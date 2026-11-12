//! The browser driver on the real WebView2 engine (V13). They need the
//! WebView2 runtime and run one at a time:
//! `cargo test --locked -p mannie-desktop-core --lib real_webview2 -- --ignored --test-threads=1`.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_tools::browser::{
    Chrome, ChromeFrame, ChromeHost, ChromeInput, ChromeNavigation, ChromeNavigationAction,
    ChromeObservation, ChromeRequest, ChromeTarget, cleanup_browser_profiles, process_running,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW,
};
use windows::core::w;
use zeroize::Zeroizing;

use super::{UiDispatch, UiThread, WebView2Session, lock, start_session};

type Task = Box<dyn FnOnce() + Send>;

/// A message loop on a thread of its own, with a hidden top-level window
/// that stands in for the main window.
struct TestUi {
    tasks: Mutex<std_mpsc::SyncSender<Task>>,
    thread: u32,
    window: isize,
}

impl TestUi {
    fn start() -> Arc<Self> {
        let (tasks, receive) = std_mpsc::sync_channel::<Task>(256);
        let (ready, started) = std_mpsc::sync_channel(1);
        std::thread::spawn(move || {
            // SAFETY: the thread is new and joins a single-threaded
            // apartment once, as WebView2 requires.
            let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            assert!(initialized.is_ok(), "{initialized:?}");
            super::surface::register_class_for_tests();
            // SAFETY: the class is registered and the arguments are plain
            // values; the window stays hidden.
            let window = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("MannieBrowserPage"),
                    w!("Mannie browser test"),
                    WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                    0,
                    0,
                    1024,
                    768,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap();
            // SAFETY: GetCurrentThreadId has no preconditions.
            let thread = unsafe { GetCurrentThreadId() };
            ready.send((thread, window.0 as isize)).unwrap();
            let mut message = MSG::default();
            loop {
                while let Ok(task) = receive.try_recv() {
                    task();
                }
                // SAFETY: `message` is writable, and the loop belongs to this
                // thread.
                let got = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
                if got.0 <= 0 {
                    break;
                }
                // SAFETY: the message came from this thread's queue.
                let _translated = unsafe { TranslateMessage(&raw const message) };
                // SAFETY: as above.
                unsafe { DispatchMessageW(&raw const message) };
            }
        });
        let (thread, window) = started.recv().unwrap();
        Arc::new(Self {
            tasks: Mutex::new(tasks),
            thread,
            window,
        })
    }
}

impl UiDispatch for TestUi {
    fn post(&self, task: Task) -> Result<(), mannie_tools::browser::ChromeError> {
        lock(&self.tasks)
            .send(task)
            .map_err(|_closed| mannie_tools::browser::ChromeError::Disconnected)?;
        // SAFETY: the thread runs a message loop; WM_APP only wakes it.
        unsafe { PostThreadMessageW(self.thread, WM_APP, WPARAM(0), LPARAM(0)) }
            .map_err(|_closed| mannie_tools::browser::ChromeError::Disconnected)
    }

    fn main_window(&self) -> Option<HWND> {
        Some(HWND(self.window as *mut std::ffi::c_void))
    }
}

/// The owner's decisions by origin; any other origin waits in `asks`.
struct Gate {
    quick: Mutex<HashMap<String, bool>>,
    asks: mpsc::Sender<(ChromeRequest, oneshot::Sender<bool>)>,
}

impl ChromeNavigation for Gate {
    fn approved(&self, request: &ChromeRequest) -> Option<bool> {
        lock(&self.quick).get(&request.origin).copied()
    }

    fn allow(&self, request: ChromeRequest) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            let (decide, answer) = oneshot::channel();
            if self.asks.send((request, decide)).await.is_err() {
                return false;
            }
            answer.await.unwrap_or(false)
        })
    }
}

struct Session {
    chrome: Arc<Chrome>,
    host: Arc<WebView2Session>,
    gate: Arc<Gate>,
    asks: mpsc::Receiver<(ChromeRequest, oneshot::Sender<bool>)>,
    root: tempfile::TempDir,
}

async fn session() -> Session {
    let ui = UiThread::with(TestUi::start());
    let root = tempfile::tempdir().unwrap();
    let (host, events) = start_session(ui, root.path()).await.unwrap();
    let (asks_sender, asks) = mpsc::channel(8);
    let gate = Arc::new(Gate {
        quick: Mutex::default(),
        asks: asks_sender,
    });
    let driver_host: Arc<dyn ChromeHost> = Arc::<WebView2Session>::clone(&host);
    let navigation: Arc<dyn ChromeNavigation> = Arc::<Gate>::clone(&gate);
    let chrome = Arc::new(Chrome::start(driver_host, events, navigation));
    Session {
        chrome,
        host,
        gate,
        asks,
        root,
    }
}

impl Session {
    fn approve(&self, origin: &str) {
        lock(&self.gate.quick).insert(origin.to_owned(), true);
    }
}

#[derive(Debug)]
struct Request {
    line: String,
    body: String,
}

/// A local site that records each request and answers with `respond`.
async fn site(
    respond: impl Fn(&str) -> String + Send + Sync + 'static,
) -> (String, mpsc::Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (requests, receive) = mpsc::channel(64);
    let respond = Arc::new(respond);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let requests = requests.clone();
            let respond = Arc::clone(&respond);
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                let head_end = loop {
                    let Ok(count) = stream.read(&mut buffer).await else {
                        return;
                    };
                    if count == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let head = String::from_utf8_lossy(&bytes[..head_end]).into_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())?
                    })
                    .unwrap_or(0);
                while bytes.len() < head_end + length {
                    let Ok(count) = stream.read(&mut buffer).await else {
                        return;
                    };
                    if count == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..count]);
                }
                let line = head.lines().next().unwrap_or_default().to_owned();
                let path = line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_owned();
                let body = String::from_utf8_lossy(&bytes[head_end..]).into_owned();
                let page = respond(&path);
                drop(requests.send(Request { line, body }).await);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                );
                drop(stream.write_all(response.as_bytes()).await);
            });
        }
    });
    (origin, receive)
}

async fn next<T>(receive: &mut mpsc::Receiver<T>) -> T {
    tokio::time::timeout(Duration::from_secs(15), receive.recv())
        .await
        .expect("in time")
        .expect("open")
}

/// The next request whose line starts with `start`, past others such as an
/// icon's.
async fn next_request(receive: &mut mpsc::Receiver<Request>, start: &str) -> Request {
    loop {
        let request = next(receive).await;
        if request.line.starts_with(start) {
            return request;
        }
    }
}

fn element(observation: &ChromeObservation, role: &str, name: &str) -> ChromeTarget {
    let node = observation
        .outline
        .iter()
        .find(|node| node.role == role && node.name == name)
        .unwrap_or_else(|| panic!("no {role} {name} in {}", observation.text));
    ChromeTarget::Element(node.reference.clone().unwrap())
}

async fn observe(chrome: &Chrome, tab: &str) -> ChromeObservation {
    for _ in 0..50 {
        if let Ok(observation) = chrome.observe(tab, true).await {
            return observation;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the page never settled");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_site_approval_click_and_replayed_form() {
    let (other, mut other_requests) =
        site(|_| "<!doctype html><title>Done</title><p>Received</p>".to_owned()).await;
    let form_target = other.clone();
    let (origin, mut requests) = site(move |_| {
        format!(
            "<!doctype html><title>Form</title>
             <button onclick=\"document.getElementById('state').textContent = 'Clicked'\">Press</button>
             <button style=\"position: absolute; left: 2400px; top: 1600px\" onclick=\"document.getElementById('state').textContent = 'Far clicked'\">Far</button>
             <p id=state>Waiting</p>
             <form method=post action=\"{form_target}/submit\"><input name=name value=value><button>Send</button></form>"
        )
    })
    .await;
    let mut session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&origin);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{origin}/form")))
        .await
        .unwrap();
    next_request(&mut requests, "GET /form").await;

    let page = observe(&chrome, &tab).await;
    assert!(page.frame.is_some());
    let press = element(&page, "button", "Press");
    chrome
        .interact(&tab, &page.tab.revision, ChromeInput::Click(press))
        .await
        .unwrap();
    let page = observe(&chrome, &tab).await;
    assert!(page.text.contains("Clicked"), "{}", page.text);

    // A target outside the viewport scrolls into view before the click.
    let far = element(&page, "button", "Far");
    chrome
        .interact(&tab, &page.tab.revision, ChromeInput::Click(far))
        .await
        .unwrap();
    let page = observe(&chrome, &tab).await;
    assert!(page.text.contains("Far clicked"), "{}", page.text);

    // The form posts to an origin nobody approved: the engine sends nothing
    // while the owner decides, then the approved request goes as it was.
    let send = element(&page, "button", "Send");
    let revision = page.tab.revision.clone();
    let clicking = Arc::clone(&chrome);
    let clicked_tab = tab.clone();
    let click = tokio::spawn(async move {
        clicking
            .interact(&clicked_tab, &revision, ChromeInput::Click(send))
            .await
    });
    let (request, decide) = next(&mut session.asks).await;
    assert_eq!(request.origin, other);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        other_requests.try_recv().is_err(),
        "a request left before approval"
    );
    decide.send(true).unwrap();
    let posted = next_request(&mut other_requests, "POST /submit").await;
    assert_eq!(posted.body, "name=value");
    click.await.unwrap().unwrap();
    let page = observe(&chrome, &tab).await;
    assert_eq!(page.tab.url, format!("{other}/submit"));

    chrome.end().await;
    assert!(!session.host.shared.folder_exists());
    drop(session.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_popups_with_and_without_an_opener() {
    let (target, mut target_requests) =
        site(|_| "<!doctype html><title>Popup</title>".to_owned()).await;
    let opener_page = target.clone();
    let (origin, _requests) = site(move |_| {
        format!(
            "<!doctype html><title>Opener</title>
             <button onclick=\"window.open('{opener_page}/with-opener')\">Open</button>
             <a href=\"{opener_page}/no-opener\" target=_blank rel=noopener>Link</a>"
        )
    })
    .await;
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&origin);
    session.approve(&target);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{origin}/")))
        .await
        .unwrap();
    let page = observe(&chrome, &tab).await;
    let open = element(&page, "button", "Open");
    chrome
        .interact(&tab, &page.tab.revision, ChromeInput::Click(open))
        .await
        .unwrap();
    next_request(&mut target_requests, "GET /with-opener").await;
    let page = observe(&chrome, &tab).await;
    let link = element(&page, "link", "Link");
    chrome
        .interact(&tab, &page.tab.revision, ChromeInput::Click(link))
        .await
        .unwrap();
    next_request(&mut target_requests, "GET /no-opener").await;
    for _ in 0..50 {
        if chrome.tabs().len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let urls: Vec<_> = chrome.tabs().into_iter().map(|tab| tab.url).collect();
    assert!(urls.contains(&format!("{target}/with-opener")), "{urls:?}");
    assert!(urls.contains(&format!("{target}/no-opener")), "{urls:?}");
    chrome.end().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_history_needs_approval() {
    let (first, _first_requests) = site(|_| "<!doctype html><title>First</title>".to_owned()).await;
    let (second, _second_requests) =
        site(|_| "<!doctype html><title>Second</title>".to_owned()).await;
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&first);
    session.approve(&second);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{first}/")))
        .await
        .unwrap();
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{second}/")))
        .await
        .unwrap();
    for _ in 0..50 {
        if chrome.tabs()[0].back.as_deref() == Some(first.as_str()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    lock(&session.gate.quick).insert(first.clone(), false);
    assert!(
        chrome
            .navigate(&tab, ChromeNavigationAction::Back)
            .await
            .is_err()
    );
    lock(&session.gate.quick).insert(first.clone(), true);
    chrome
        .navigate(&tab, ChromeNavigationAction::Back)
        .await
        .unwrap();
    assert_eq!(chrome.tabs()[0].url, format!("{first}/"));
    chrome.end().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_ending_a_session_ends_its_processes_and_folder() {
    let (origin, _requests) = site(|_| "<!doctype html><title>Page</title>".to_owned()).await;
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&origin);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{origin}/")))
        .await
        .unwrap();
    let shared = &session.host.shared;
    assert!(!lock(&shared.folder).as_ref().unwrap().has_debugging_port());
    let processes: Vec<u32> = lock(&shared.processes).iter().copied().collect();
    assert_ne!(processes.len(), 0);
    chrome.end().await;
    for process in processes {
        assert!(
            !process_running(process),
            "process {process} outlived its session"
        );
    }
    assert!(!shared.folder_exists());
    assert_eq!(sessions_in(session.root.path()), 0);
}

fn sessions_in(root: &Path) -> usize {
    std::fs::read_dir(root).map_or(0, |entries| {
        entries
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("session-"))
            .count()
    })
}

/// Runs in a child process that the next test kills.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "started by real_webview2_a_killed_host_leaves_no_process"]
async fn real_webview2_victim() {
    let Ok(address) = std::env::var("MANNIE_WEBVIEW2_VICTIM_SITE") else {
        return;
    };
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&address);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{address}/")))
        .await
        .unwrap();
    let processes: Vec<String> = lock(&session.host.shared.processes)
        .iter()
        .map(u32::to_string)
        .collect();
    println!("ROOT {}", session.root.path().display());
    println!("PROCESSES {}", processes.join(","));
    println!("READY");
    // The parent kills this process while the session runs.
    tokio::time::sleep(Duration::from_secs(120)).await;
    drop(session);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_a_killed_host_leaves_no_process() {
    let (origin, _requests) = site(|_| "<!doctype html><title>Page</title>".to_owned()).await;
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "browser::webview2::tests::real_webview2_victim",
            "--ignored",
            "--nocapture",
        ])
        .env("MANNIE_WEBVIEW2_VICTIM_SITE", &origin)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut root = None;
    let mut processes = Vec::new();
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line.unwrap();
        if let Some(path) = line.strip_prefix("ROOT ") {
            root = Some(std::path::PathBuf::from(path));
        }
        if let Some(list) = line.strip_prefix("PROCESSES ") {
            processes = list
                .split(',')
                .filter_map(|id| id.parse::<u32>().ok())
                .collect();
        }
        if line == "READY" {
            break;
        }
    }
    assert_ne!(processes.len(), 0);
    child.kill().unwrap();
    child.wait().unwrap();
    // The job closes with the host's handles; the engine's other processes
    // follow their browser process.
    for _ in 0..100 {
        if processes.iter().all(|process| !process_running(*process)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for process in &processes {
        assert!(
            !process_running(*process),
            "process {process} outlived its host"
        );
    }
    // The next start removes the folder, whose lock died with the host.
    let root = root.unwrap();
    cleanup_browser_profiles(&root);
    assert_eq!(sessions_in(&root), 0);
    drop(std::fs::remove_dir_all(root));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_keys_reach_a_page_without_the_system_focus() {
    let (origin, _requests) = site(|_| {
        "<!doctype html><title>Keys</title>
         <form onsubmit=\"event.preventDefault(); document.getElementById('state').textContent = 'Submitted ' + document.querySelector('input').value\">
           <input aria-label=Task onkeydown=\"document.getElementById('keys').textContent += event.key + ' '\">
         </form>
         <p id=keys></p><p id=state>Waiting</p>"
            .to_owned()
    })
    .await;
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&origin);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{origin}/")))
        .await
        .unwrap();
    // The page's window is disabled and has no focus, as under agent control.
    let page = observe(&chrome, &tab).await;
    let field = element(&page, "textbox", "Task");
    chrome
        .interact(
            &tab,
            &page.tab.revision,
            ChromeInput::Fill {
                target: field,
                text: "Buy milk".to_owned(),
                protected: false,
            },
        )
        .await
        .unwrap();
    let page = observe(&chrome, &tab).await;
    chrome
        .interact(
            &tab,
            &page.tab.revision,
            ChromeInput::Press {
                key: "Enter".to_owned(),
            },
        )
        .await
        .unwrap();
    let page = observe(&chrome, &tab).await;
    assert!(page.text.contains("Enter"), "{}", page.text);
    assert!(page.text.contains("Submitted Buy milk"), "{}", page.text);
    chrome.end().await;
}

/// A list sorted with HTML drag and drop, as most to-do apps do it, and one
/// sorted with pointer events, as libraries such as dnd-kit do it. Each
/// records the order it ends in.
const SORTABLE_LISTS: &str = r#"<!doctype html><title>Sorting</title>
<ul id=html style="list-style:none;margin:0;padding:0;width:300px">
  <li draggable=true data-id=a style="height:48px">Alpha</li>
  <li draggable=true data-id=b style="height:48px">Bravo</li>
  <li draggable=true data-id=c style="height:48px">Charlie</li>
</ul>
<ul id=pointer style="list-style:none;margin:0;padding:0;width:300px;touch-action:none">
  <li data-id=x style="height:48px">Xray</li>
  <li data-id=y style="height:48px">Yankee</li>
  <li data-id=z style="height:48px">Zulu</li>
</ul>
<p id=html-order></p><p id=pointer-order></p>
<script>
  const order = (list) => [...list.children].map((item) => item.dataset.id).join('');
  const html = document.getElementById('html');
  let dragged = null;
  html.addEventListener('dragstart', (event) => {
    dragged = event.target.closest('li');
    event.dataTransfer.effectAllowed = 'move';
    event.dataTransfer.setData('text/plain', dragged.dataset.id);
  });
  html.addEventListener('dragenter', (event) => { if (dragged) event.preventDefault(); });
  html.addEventListener('dragover', (event) => {
    if (!dragged) return;
    event.preventDefault();
    const over = event.target.closest('li');
    if (!over || over === dragged) return;
    const box = over.getBoundingClientRect();
    html.insertBefore(dragged, event.clientY > box.top + box.height / 2 ? over.nextElementSibling : over);
  });
  html.addEventListener('drop', (event) => { if (dragged) event.preventDefault(); });
  html.addEventListener('dragend', () => {
    dragged = null;
    document.getElementById('html-order').textContent = 'HTML order ' + order(html);
  });
  const pointer = document.getElementById('pointer');
  let held = null;
  pointer.addEventListener('pointerdown', (event) => { held = event.target.closest('li'); });
  pointer.addEventListener('pointermove', (event) => {
    if (!held) return;
    const over = document.elementFromPoint(event.clientX, event.clientY)?.closest('#pointer li');
    if (!over || over === held) return;
    const box = over.getBoundingClientRect();
    pointer.insertBefore(held, event.clientY > box.top + box.height / 2 ? over.nextElementSibling : over);
  });
  pointer.addEventListener('pointerup', () => {
    held = null;
    document.getElementById('pointer-order').textContent = 'Pointer order ' + order(pointer);
  });
</script>"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_an_agent_drags_to_sort_lists() {
    let (origin, _requests) = site(|_| SORTABLE_LISTS.to_owned()).await;
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&origin);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{origin}/")))
        .await
        .unwrap();
    // The page's window is disabled and has no focus, as under agent control.
    for (from, to, expected) in [
        // The center of a row is its upper half, so the dragged row lands
        // before it.
        ("Alpha", "Charlie", "HTML order bac"),
        ("Xray", "Zulu", "Pointer order yxz"),
    ] {
        let page = observe(&chrome, &tab).await;
        let from = element(&page, "listitem", from);
        let to = element(&page, "listitem", to);
        chrome
            .interact(&tab, &page.tab.revision, ChromeInput::Drag { from, to })
            .await
            .unwrap();
        let page = observe(&chrome, &tab).await;
        assert!(page.text.contains(expected), "{}", page.text);
    }
    chrome.end().await;
}

const REPAINTING_CANVAS: &str = r#"<!doctype html><title>Repainting canvas</title>
<canvas id=surface style="position:fixed;inset:0;width:100%;height:100%;touch-action:none"></canvas>
<p id=counts style="position:fixed;pointer-events:none"></p>
<script>
  const canvas = document.getElementById('surface');
  const context = canvas.getContext('2d', { alpha: false });
  let frame = 0;
  function paint() {
    const color = (frame++ * 1597) % 4096;
    const red = 16 * (color & 15);
    const green = 16 * ((color >> 4) & 15);
    const blue = 16 * ((color >> 8) & 15);
    context.fillStyle = `rgb(${red}, ${green}, ${blue})`;
    context.fillRect(0, 0, canvas.width, canvas.height);
    requestAnimationFrame(paint);
  }
  requestAnimationFrame(paint);

  let downs = 0;
  let ups = 0;
  let clicks = 0;
  let drags = 0;
  let startedOnLeft = false;
  const onLeft = (event) => event.clientX < innerWidth / 2;
  canvas.addEventListener('pointerdown', (event) => {
    downs += 1;
    startedOnLeft = onLeft(event);
  });
  canvas.addEventListener('pointerup', (event) => {
    ups += 1;
    if (startedOnLeft && onLeft(event)) clicks += 1;
    if (startedOnLeft && !onLeft(event)) drags += 1;
    document.getElementById('counts').textContent =
      `Downs ${downs}, ups ${ups}, clicks ${clicks}, drags ${drags}`;
  });
</script>"#;

async fn evaluate(host: &WebView2Session, tab: &str, expression: &str) -> Value {
    let parameters = json!({
        "expression": expression,
        "awaitPromise": true,
        "returnByValue": true,
    });
    let parameters = Zeroizing::new(parameters.to_string());
    let reply = host.send(tab, "Runtime.evaluate", parameters).unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(5), reply)
        .await
        .expect("script completed")
        .unwrap()
        .unwrap();
    assert!(answer["exceptionDetails"].is_null(), "{answer}");
    answer["result"]["value"].clone()
}

async fn wait_for_canvas_repaint(host: &WebView2Session, tab: &str) {
    // Two frame callbacks allow a paint to finish after the observation.
    evaluate(
        host,
        tab,
        "new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)))",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_accepts_points_on_a_repainting_canvas() {
    let (origin, _requests) = site(|_| REPAINTING_CANVAS.to_owned()).await;
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&origin);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{origin}/")))
        .await
        .unwrap();

    let before = observe(&chrome, &tab).await;
    wait_for_canvas_repaint(&session.host, &tab).await;
    let page = observe(&chrome, &tab).await;
    let frame = page.frame.as_ref().unwrap();
    assert_ne!(
        before.frame.unwrap().jpeg,
        frame.jpeg,
        "canvas pixels changed"
    );

    let points = |frame: &ChromeFrame| {
        let x = i32::try_from(frame.width / 4).unwrap();
        let y = i32::try_from(frame.height / 2).unwrap();
        (
            ChromeTarget::Point { x, y },
            ChromeTarget::Point { x: 3 * x, y },
        )
    };
    let (left, _) = points(frame);
    wait_for_canvas_repaint(&session.host, &tab).await;
    chrome
        .interact(&tab, &page.tab.revision, ChromeInput::Click(left))
        .await
        .unwrap();

    let page = observe(&chrome, &tab).await;
    assert!(
        page.text.contains("Downs 1, ups 1, clicks 1, drags 0"),
        "{}",
        page.text
    );
    let (from, to) = points(page.frame.as_ref().unwrap());
    wait_for_canvas_repaint(&session.host, &tab).await;
    chrome
        .interact(&tab, &page.tab.revision, ChromeInput::Drag { from, to })
        .await
        .unwrap();

    let page = chrome.observe(&tab, false).await.unwrap();
    assert!(
        page.text.contains("Downs 2, ups 2, clicks 1, drags 1"),
        "{}",
        page.text
    );
    chrome.end().await;
}

const POINTER_CAPTURE: &str = r#"<!doctype html><title>Canvas pointer capture</title>
<canvas id=surface style="position:fixed;inset:0 auto 0 0;width:40vw;height:100vh;background:blue"></canvas>
<p id=events style="position:fixed;right:0;top:0;pointer-events:none">Waiting for drag</p>
<script>
  const canvas = document.getElementById('surface');
  const events = [];
  let movedOutside = false;
  const outside = (event) => event.clientX >= canvas.getBoundingClientRect().right;
  canvas.addEventListener('pointerdown', (event) => {
    events.push('Down');
    canvas.setPointerCapture(event.pointerId);
  });
  canvas.addEventListener('gotpointercapture', () => events.push('captured'));
  canvas.addEventListener('pointermove', (event) => {
    if (!movedOutside && outside(event) && canvas.hasPointerCapture(event.pointerId)) {
      movedOutside = true;
      events.push('moved outside');
    }
  });
  canvas.addEventListener('pointerup', (event) => {
    if (outside(event) && canvas.hasPointerCapture(event.pointerId)) {
      events.push('released outside');
    }
  });
  canvas.addEventListener('lostpointercapture', () => {
    events.push('capture lost');
    document.getElementById('events').textContent = events.join(', ');
  });
</script>"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_drag_honors_canvas_pointer_capture() {
    let (origin, _requests) = site(|_| POINTER_CAPTURE.to_owned()).await;
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&origin);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{origin}/")))
        .await
        .unwrap();

    let page = observe(&chrome, &tab).await;
    let frame = page.frame.as_ref().unwrap();
    let x = i32::try_from(frame.width / 4).unwrap();
    let y = i32::try_from(frame.height / 2).unwrap();
    let from = ChromeTarget::Point { x, y };
    let to = ChromeTarget::Point { x: 3 * x, y };
    chrome
        .interact(&tab, &page.tab.revision, ChromeInput::Drag { from, to })
        .await
        .unwrap();

    let page = chrome.observe(&tab, false).await.unwrap();
    assert!(
        page.text
            .contains("Down, captured, moved outside, released outside, capture lost"),
        "{}",
        page.text
    );
    chrome.end().await;
}

const SCROLLABLE_PAGE: &str = "<!doctype html><title>Scrollable page</title>
<style>
  html { overflow: scroll; }
  html::-webkit-scrollbar { width: 24px; height: 24px; background: #eee; }
  html::-webkit-scrollbar-thumb { background: #333; }
  main { width: 200vw; height: 200vh; }
</style>
<main>Scrollable content</main>";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "V13 needs the WebView2 runtime"]
async fn real_webview2_reports_the_jpeg_dimensions_with_scrollbars() {
    let (origin, _requests) = site(|_| SCROLLABLE_PAGE.to_owned()).await;
    let session = session().await;
    let chrome = Arc::clone(&session.chrome);
    let tab = chrome.open_tab().await.unwrap();
    session.approve(&origin);
    chrome
        .navigate(&tab, ChromeNavigationAction::Go(format!("{origin}/")))
        .await
        .unwrap();

    let page = observe(&chrome, &tab).await;
    let frame = page.frame.as_ref().unwrap();
    let data_url = json!(format!("data:image/jpeg;base64,{}", frame.jpeg));
    let expression = format!(
        "(async () => {{
            const image = new Image();
            image.src = {data_url};
            await image.decode();
            return {{
                dimensions: [image.naturalWidth, image.naturalHeight],
                vertical: innerWidth > document.documentElement.clientWidth,
                horizontal: innerHeight > document.documentElement.clientHeight
            }};
        }})()"
    );
    let result = evaluate(&session.host, &tab, &expression).await;
    assert_eq!(result["vertical"].as_bool(), Some(true), "{result}");
    assert_eq!(result["horizontal"].as_bool(), Some(true), "{result}");
    assert_eq!(
        json!([frame.width, frame.height]),
        result["dimensions"],
        "reported dimensions must match the actual JPEG with both gutters"
    );
    chrome.end().await;
}
