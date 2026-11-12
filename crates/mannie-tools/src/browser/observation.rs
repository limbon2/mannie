//! A bounded model observation built from Chromium's accessibility tree.

use std::collections::{HashMap, HashSet};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use mannie_types::images::jpeg;
use mannie_types::tools::MAX_TOOL_IMAGE_SIDE;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use super::driver::{Chrome, Context, Driver, Page, State, Viewport, redact};
use super::{
    ChromeError, ChromeFrame, ChromeNode, ChromeObservation, ChromeScreenshotStatus, lock,
};

const MAX_NODES: usize = 160;
const MAX_NODE_NAME: usize = 384;
const MAX_TEXT_BYTES: usize = 12_000;
const MAX_IMAGE_BYTES: usize = 2_000_000;

impl Chrome {
    /// Reads the loaded page's semantics and, when permitted, its viewport.
    pub async fn observe(
        &self,
        tab: &str,
        screenshot: bool,
    ) -> Result<ChromeObservation, ChromeError> {
        let context = self.inner.context(tab, None)?;
        if lock(&self.inner.state)
            .tabs
            .get(tab)
            .is_some_and(|page| !page.settled())
        {
            return Err(ChromeError::Stale);
        }
        let tree = self
            .inner
            .command(
                &context,
                "Accessibility.getFullAXTree",
                json!({ "frameId": context.main_frame }),
            )
            .await?;
        let nodes = tree["nodes"].as_array().ok_or(ChromeError::Protocol)?;
        self.inner.revalidate(&context)?;
        let viewport = self.inner.layout(&context).await?;
        let (mut screenshot, mut frame, viewport) = self
            .inner
            .observe_frame(&context, screenshot, viewport)
            .await?;
        self.inner.revalidate(&context)?;

        let mut state = lock(&self.inner.state);
        let State {
            tabs,
            protected_values,
            ..
        } = &mut *state;
        let page = tabs.get_mut(tab).ok_or(ChromeError::NoTab)?;
        if !page.is_current(&context) || !page.settled() {
            return Err(ChromeError::Stale);
        }
        if frame.is_some() && page.sensitive_documents.contains(&page.document) {
            screenshot = ChromeScreenshotStatus::Withheld;
            frame = None;
        }
        page.invalidate();
        page.needs_observation = false;
        page.viewport = viewport;
        page.screenshot_size = frame.as_ref().map(|frame| (frame.width, frame.height));
        let (outline, text) = semantics(page, nodes, protected_values);
        let mut tab = page.tab.clone();
        if tab.title.is_empty()
            && let Some(node) = outline.first()
            && node.role == "RootWebArea"
        {
            tab.title.clone_from(&node.name);
        }
        Ok(ChromeObservation {
            tab,
            frame,
            screenshot,
            outline,
            text,
        })
    }
}

impl Driver {
    async fn observe_frame(
        &self,
        context: &Context,
        requested: bool,
        viewport: Viewport,
    ) -> Result<(ChromeScreenshotStatus, Option<ChromeFrame>, Viewport), ChromeError> {
        use ChromeScreenshotStatus::{NotRequested, Returned, Unavailable, Withheld};

        if !requested {
            return Ok((NotRequested, None, viewport));
        }
        {
            let state = lock(&self.state);
            let page = state.tabs.get(&context.id).ok_or(ChromeError::NoTab)?;
            if page.sensitive_documents.contains(&page.document) {
                return Ok((Withheld, None, viewport));
            }
        }
        let capture = async {
            let frame = self.frame(context).await?;
            let current = self.layout(context).await?;
            if current == viewport {
                Ok((Returned, Some(frame), current))
            } else {
                Ok((Unavailable, None, current))
            }
        };
        match capture.await {
            Ok(result) => Ok(result),
            Err(ChromeError::Limit | ChromeError::Protocol | ChromeError::Stale) => {
                // A recoverable image failure can leave the semantic result usable.
                // The caller revalidates the document before exposing either result.
                Ok((Unavailable, None, viewport))
            }
            Err(error) => Err(error),
        }
    }

    /// The page's visual viewport in CSS pixels and its device pixel ratio.
    pub(super) async fn layout(&self, context: &Context) -> Result<Viewport, ChromeError> {
        let metrics = self
            .command(context, "Page.getLayoutMetrics", json!({}))
            .await?;
        let css = &metrics["cssVisualViewport"];
        let measure = |key| css[key].as_f64().ok_or(ChromeError::Protocol);
        let width = measure("clientWidth")?;
        let height = measure("clientHeight")?;
        let device_width = metrics["visualViewport"]["clientWidth"]
            .as_f64()
            .ok_or(ChromeError::Protocol)?;
        // Chromium reports the layout scroll and the visual viewport's offset
        // within it separately. Input uses the visual viewport as its origin.
        let document_x = measure("pageX")? + measure("offsetX")?;
        let document_y = measure("pageY")? + measure("offsetY")?;
        let page_scale = measure("scale")?;
        let scale = (device_width / width) * page_scale;
        if !(1.0..=20_000.0).contains(&width)
            || !(1.0..=20_000.0).contains(&height)
            || !scale.is_finite()
            || (width.min(height) * scale).round() < 1.0
            || (width.max(height) * scale).round() > f64::from(u32::MAX)
            || document_x.abs() >= 1.0e9
            || document_y.abs() >= 1.0e9
        {
            return Err(ChromeError::Stale);
        }
        Ok(Viewport {
            width,
            height,
            scale,
            document_x,
            document_y,
        })
    }

    /// Captures the visible page without a clip. The encoded image can also
    /// include scrollbar gutters outside the CSS viewport.
    async fn frame(&self, context: &Context) -> Result<ChromeFrame, ChromeError> {
        let response = self.command(
            context,
            "Page.captureScreenshot",
            json!({ "format": "jpeg", "quality": 65, "fromSurface": true, "captureBeyondViewport": false }),
        ).await?;
        let jpeg = response["data"].as_str().ok_or(ChromeError::Protocol)?;
        if jpeg.len() > MAX_IMAGE_BYTES {
            return Err(ChromeError::Limit);
        }
        let bytes = STANDARD
            .decode(jpeg)
            .map_err(|_invalid| ChromeError::Protocol)?;
        let (width, height) = jpeg::dimensions(&bytes).ok_or(ChromeError::Protocol)?;
        if width > MAX_TOOL_IMAGE_SIDE || height > MAX_TOOL_IMAGE_SIDE {
            return Err(ChromeError::Limit);
        }
        Ok(ChromeFrame {
            jpeg: jpeg.to_owned(),
            width,
            height,
        })
    }
}

fn semantics(
    page: &mut Page,
    nodes: &[Value],
    protected_values: &[Zeroizing<String>],
) -> (Vec<ChromeNode>, String) {
    let editing = editing_nodes(nodes);
    let mut checkbox_names = HashMap::new();
    for node in nodes.iter().filter_map(observed_element) {
        if node.named_checkbox {
            *checkbox_names.entry(node.signature).or_insert(0usize) += 1;
        }
    }
    let by_id: HashMap<&str, &Value> = nodes
        .iter()
        .filter_map(|node| Some((node["nodeId"].as_str()?, node)))
        .collect();
    let mut outline = Vec::new();
    let mut text = String::new();
    for node in nodes {
        if node["ignored"].as_bool() == Some(true) || nested_in_editor(node, &editing) {
            continue;
        }
        let role = role(node);
        let own_name = node["name"]["value"].as_str().unwrap_or_default();
        // A list item, row or cell has no accessible name of its own, so an
        // agent tells them apart, to drag one, by the text they show.
        let shown_name = (own_name.is_empty() && takes_name_from_text(role))
            .then(|| descendant_text(node, &by_id));
        let name = redact(shown_name.as_deref().unwrap_or(own_name), protected_values);
        if role == "StaticText" {
            append_text(&mut text, &name);
            continue;
        }
        if outline.len() >= MAX_NODES
            || matches!(role, "" | "InlineTextBox")
            || name.is_empty() && !interactive(role)
        {
            continue;
        }
        let element = observed_element(node);
        let checked = match element.and_then(|target| target.checked) {
            Some(Checked::False) => Some(false),
            Some(Checked::True) => Some(true),
            Some(Checked::Mixed) | None => None,
        };
        let reference = element.map(|mut target| {
            let backend = target.backend;
            let reference = format!("{}-{backend}", page.revision);
            target.can_rebind = target.named_checkbox
                && target.checked.is_some()
                && checkbox_names.get(&target.signature) == Some(&1);
            page.elements.insert(reference.clone(), target);
            reference
        });
        outline.push(ChromeNode {
            role: role.chars().take(64).collect(),
            name: name.chars().take(MAX_NODE_NAME).collect(),
            reference,
            checked,
        });
    }
    (outline, text)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ObservedElement {
    pub(super) backend: i64,
    signature: [u8; 32],
    checked: Option<Checked>,
    disabled: bool,
    named_checkbox: bool,
    pub(super) can_rebind: bool,
}

impl ObservedElement {
    pub(super) fn same_name(self, other: Self) -> bool {
        self.signature == other.signature
    }

    pub(super) fn same_identity(self, other: Self) -> bool {
        !self.disabled && !other.disabled && self.same_name(other) && self.checked == other.checked
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Checked {
    False,
    True,
    Mixed,
}

pub(super) fn observed_element(node: &Value) -> Option<ObservedElement> {
    if node["ignored"].as_bool() == Some(true) {
        return None;
    }
    let backend = node["backendDOMNodeId"].as_i64()?;
    let role = role(node);
    if !interactive(role) {
        return None;
    }
    let name = node["name"]["value"].as_str().unwrap_or_default();
    // Keep a semantic identity for rerenders without retaining page text in
    // input state. The name must be meaningful to rebind a checkbox.
    let mut signature = Sha256::new();
    signature.update(role.as_bytes());
    signature.update([0]);
    signature.update(name.as_bytes());
    let is_checkbox = matches!(role, "checkbox" | "checkBox");
    let checked = if is_checkbox {
        checkbox_state(node)
    } else {
        None
    };
    Some(ObservedElement {
        backend,
        signature: signature.finalize().into(),
        checked,
        disabled: ax_property(node, "disabled").and_then(Value::as_bool) == Some(true),
        named_checkbox: is_checkbox && !name.trim().is_empty(),
        can_rebind: false,
    })
}

fn ax_property<'a>(node: &'a Value, name: &str) -> Option<&'a Value> {
    let property = node["properties"]
        .as_array()?
        .iter()
        .find(|property| property["name"].as_str() == Some(name))?;
    Some(&property["value"]["value"])
}

fn checkbox_state(node: &Value) -> Option<Checked> {
    match ax_property(node, "checked")? {
        Value::Bool(false) => Some(Checked::False),
        Value::Bool(true) => Some(Checked::True),
        Value::String(value) => match value.as_str() {
            "false" => Some(Checked::False),
            "true" => Some(Checked::True),
            "mixed" => Some(Checked::Mixed),
            _ => None,
        },
        _ => None,
    }
}

fn role(node: &Value) -> &str {
    node["role"]["value"].as_str().unwrap_or_default()
}

fn editable(role: &str) -> bool {
    matches!(
        role,
        "textbox"
            | "textField"
            | "textFieldWithComboBox"
            | "searchbox"
            | "spinbutton"
            | "combobox"
            | "comboBox"
    )
}

fn editing_nodes(nodes: &[Value]) -> HashSet<&str> {
    let parents: HashMap<&str, &str> = nodes
        .iter()
        .filter_map(|node| Some((node["nodeId"].as_str()?, node["parentId"].as_str()?)))
        .collect();
    let editors: HashSet<&str> = nodes
        .iter()
        .filter_map(|node| {
            editable(role(node))
                .then(|| node["nodeId"].as_str())
                .flatten()
        })
        .collect();
    nodes
        .iter()
        .filter_map(|node| {
            let id = node["nodeId"].as_str()?;
            let mut parent = parents.get(id).copied();
            for _ in 0..64 {
                let id_of_parent = parent?;
                if editors.contains(id_of_parent) {
                    return Some(id);
                }
                parent = parents.get(id_of_parent).copied();
            }
            Some(id)
        })
        .collect()
}

fn nested_in_editor(node: &Value, editing: &HashSet<&str>) -> bool {
    node["nodeId"]
        .as_str()
        .is_some_and(|id| editing.contains(id))
}

fn takes_name_from_text(role: &str) -> bool {
    matches!(
        role,
        "listitem"
            | "listItem"
            | "row"
            | "treeitem"
            | "treeItem"
            | "cell"
            | "gridcell"
            | "gridCell"
    )
}

/// The text that a node's descendants show, bounded, for a node without a
/// name of its own.
fn descendant_text(node: &Value, by_id: &HashMap<&str, &Value>) -> String {
    let mut text = String::new();
    let mut pending: Vec<&Value> = vec![node];
    let mut visited = 0;
    while let Some(current) = pending.pop() {
        visited += 1;
        if visited > 64 || text.len() > MAX_NODE_NAME {
            break;
        }
        if role(current) == "StaticText" {
            let shown = current["name"]["value"].as_str().unwrap_or_default().trim();
            if !shown.is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(shown);
            }
            continue;
        }
        let children = current["childIds"].as_array().into_iter().flatten();
        let children: Vec<&Value> = children
            .filter_map(|id| by_id.get(id.as_str()?).copied())
            .collect();
        pending.extend(children.into_iter().rev());
    }
    text
}

fn interactive(role: &str) -> bool {
    editable(role)
        || matches!(
            role,
            "button"
                | "link"
                | "checkbox"
                | "checkBox"
                | "radio"
                | "radioButton"
                | "listbox"
                | "listBox"
                | "menuitem"
                | "menuItem"
                | "option"
                | "switch"
                | "tab"
                | "togglebutton"
                | "toggleButton"
                | "DisclosureTriangle"
                | "popUpButton"
                | "slider"
                | "date"
                | "time"
                // Rows and items: what an agent drags to sort a list or a
                // table, and drops onto.
                | "listitem"
                | "listItem"
                | "row"
                | "treeitem"
                | "treeItem"
                | "cell"
                | "gridcell"
                | "gridCell"
        )
}

fn append_text(text: &mut String, name: &str) {
    if name.is_empty() || text.len() >= MAX_TEXT_BYTES {
        return;
    }
    if !text.is_empty() {
        text.push('\n');
    }
    let limit = name.floor_char_boundary(MAX_TEXT_BYTES - text.len());
    text.push_str(&name[..limit]);
}
