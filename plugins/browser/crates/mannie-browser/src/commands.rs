//! Generated IPC for the Browser pane: its status, the owner's controls, the
//! placement of the native page view, session credentials and certificate exceptions.

use std::sync::Arc;

use mannie_plugin_sdk::contracts::ConversationId;
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, Handlers, InputRefusal, Interface, Validate,
};
use mannie_plugin_sdk::services::browser::{
    BrowserApi, BrowserCertificateException, BrowserControl, BrowserCredential, BrowserError,
    BrowserFrame, BrowserPlacement, BrowserRect, BrowserStatus,
};
use serde::Deserialize;
use specta::Type;

/// The most clip rectangles that one placement carries.
const MAX_CLIPS: usize = 16;
/// The largest coordinate or size of a placement, in CSS pixels.
const MAX_EXTENT: f64 = 32_768.0;

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationInput {
    conversation_id: ConversationId,
}

impl Validate for ConversationInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        Ok(())
    }
}

fn valid_tab(tab: &str) -> bool {
    !tab.is_empty() && tab.len() <= 128
}

fn validate_tab(tab: Option<&str>) -> Result<(), InputRefusal> {
    tab.is_none_or(valid_tab)
        .then_some(())
        .ok_or_else(|| InputRefusal::new("tabId"))
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TabInput {
    conversation_id: ConversationId,
    tab_id: String,
}

impl Validate for TabInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_tab(Some(&self.tab_id))
    }
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PlaceInput {
    conversation_id: ConversationId,
    placement: BrowserPlacement,
}

fn valid_rect(rect: &BrowserRect) -> bool {
    let coordinate = |value: f64| value.is_finite() && value.abs() <= MAX_EXTENT;
    let size = |value: f64| value.is_finite() && (0.0..=MAX_EXTENT).contains(&value);
    coordinate(rect.x) && coordinate(rect.y) && size(rect.width) && size(rect.height)
}

impl Validate for PlaceInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        let placement = &self.placement;
        validate_tab(Some(&placement.tab_id))?;
        let valid = valid_rect(&placement.rect)
            && placement.scale.is_finite()
            && (0.25..=8.0).contains(&placement.scale)
            && placement.clip.len() <= MAX_CLIPS
            && placement.clip.iter().all(valid_rect);
        valid
            .then_some(())
            .ok_or_else(|| InputRefusal::new("placement"))
    }
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ControlInput {
    conversation_id: ConversationId,
    tab_id: Option<String>,
    action: BrowserControl,
}

impl Validate for ControlInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_tab(self.tab_id.as_deref())?;
        let valid = match &self.action {
            BrowserControl::Navigate { url } => !url.is_empty() && url.len() <= 8192,
            BrowserControl::SelectTab { tab_id } | BrowserControl::CloseTab { tab_id } => {
                valid_tab(tab_id)
            }
            _ => true,
        };
        valid
            .then_some(())
            .ok_or_else(|| InputRefusal::new("action"))
    }
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CredentialInput {
    conversation_id: ConversationId,
    action: BrowserCredential,
}

impl Validate for CredentialInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        let valid = match &self.action {
            BrowserCredential::Set {
                origin,
                label,
                value,
            } => {
                !origin.is_empty()
                    && origin.len() <= 512
                    && !label.trim().is_empty()
                    && label.len() <= 80
                    && !value.is_empty()
                    && value.len() <= 8192
            }
            BrowserCredential::Remove { reference } => {
                !reference.is_empty() && reference.len() <= 80
            }
        };
        valid
            .then_some(())
            .ok_or_else(|| InputRefusal::new("action"))
    }
}

#[derive(Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustCertificateInput {
    conversation_id: ConversationId,
    session_id: String,
    hostname: String,
    fingerprint: String,
}

impl Validate for TrustCertificateInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_certificate_field(&self.session_id, 128, "sessionId")?;
        validate_pin(&self.hostname, &self.fingerprint)
    }
}

fn validate_certificate_field(
    value: &str,
    limit: usize,
    field: &'static str,
) -> Result<(), InputRefusal> {
    (!value.trim().is_empty() && value.len() <= limit)
        .then_some(())
        .ok_or_else(|| InputRefusal::new(field))
}

fn validate_pin(hostname: &str, fingerprint: &str) -> Result<(), InputRefusal> {
    validate_certificate_field(hostname, 1024, "hostname")?;
    validate_certificate_field(fingerprint, 128, "fingerprint")
}

#[derive(Debug, Deserialize, Type)]
#[serde(deny_unknown_fields)]
struct SavedCertificateInput {
    hostname: String,
    fingerprint: String,
}

impl Validate for SavedCertificateInput {
    fn validate(&self) -> Result<(), InputRefusal> {
        validate_pin(&self.hostname, &self.fingerprint)
    }
}

macro_rules! command {
    ($spec:ident, $name:literal, $docs:literal, $input:ty, $output:ty) => {
        enum $spec {}

        impl CommandSpec for $spec {
            const NAME: &'static str = $name;
            const DOCS: &'static str = $docs;
            type Input = $input;
            type Output = $output;
            type Error = BrowserError;
        }
    };
}

command!(
    Status,
    "browser_status",
    "Read the conversation's browser state.",
    ConversationInput,
    BrowserStatus
);
command!(
    Place,
    "browser_place",
    "Place a tab's page over the pane's placeholder, or hide it.",
    PlaceInput,
    ()
);
command!(
    Focus,
    "browser_focus",
    "Move keyboard focus into a tab's page.",
    TabInput,
    ()
);
command!(
    Still,
    "browser_still",
    "Read a still image of a tab's page for the pane, while an overlay hides the page.",
    TabInput,
    Option<BrowserFrame>
);
command!(
    Control,
    "browser_control",
    "Control the conversation's browser.",
    ControlInput,
    ()
);
command!(
    Credential,
    "browser_credential",
    "Change a protected browser session credential.",
    CredentialInput,
    ()
);
command!(
    TrustCertificate,
    "browser_trust_certificate",
    "Trust a self-signed server certificate for the current Browser session.",
    TrustCertificateInput,
    ()
);
command!(
    SavedCertificates,
    "browser_saved_certificates",
    "List the owner's saved Browser certificate pins.",
    (),
    Vec<BrowserCertificateException>
);
command!(
    SaveCertificate,
    "browser_save_certificate",
    "Save an owner-confirmed certificate pin across Browser sessions.",
    SavedCertificateInput,
    ()
);
command!(
    ForgetCertificate,
    "browser_forget_certificate",
    "Forget the owner's exact saved Browser certificate pin.",
    SavedCertificateInput,
    ()
);

/// `browser_follow`: the conversation's browser status, first as it is and
/// then after each change. While it is open, the conversation's pages may
/// show over the app.
enum Follow {}

impl ChannelSpec for Follow {
    const NAME: &'static str = "browser_follow";
    const DOCS: &'static str =
        "Follow the conversation's browser status. Its pages show only while a pane follows it.";
    type Input = ConversationInput;
    type Item = BrowserStatus;
    type Error = BrowserError;
}

pub(crate) fn interface() -> Interface {
    Interface::new()
        .command::<Status>()
        .command::<Place>()
        .command::<Focus>()
        .command::<Still>()
        .command::<Control>()
        .command::<Credential>()
        .command::<TrustCertificate>()
        .command::<SavedCertificates>()
        .command::<SaveCertificate>()
        .command::<ForgetCertificate>()
        .channel::<Follow>()
}

pub(crate) fn handlers(browser: &Arc<dyn BrowserApi>) -> Handlers {
    let status = Arc::clone(browser);
    let place = Arc::clone(browser);
    let focus = Arc::clone(browser);
    let still = Arc::clone(browser);
    let control = Arc::clone(browser);
    let credential = Arc::clone(browser);
    let trust_certificate = Arc::clone(browser);
    let saved_certificates = Arc::clone(browser);
    let save_certificate = Arc::clone(browser);
    let forget_certificate = Arc::clone(browser);
    let follow = Arc::clone(browser);
    Handlers::new()
        .command::<Status, _, _>(move |input: ConversationInput| {
            status.status(input.conversation_id)
        })
        .command::<Place, _, _>(move |input: PlaceInput| {
            place.place(input.conversation_id, input.placement)
        })
        .command::<Focus, _, _>(move |input: TabInput| {
            focus.focus(input.conversation_id, input.tab_id)
        })
        .command::<Still, _, _>(move |input: TabInput| {
            still.still(input.conversation_id, input.tab_id)
        })
        .command::<Control, _, _>(move |input: ControlInput| {
            control.control(input.conversation_id, input.tab_id, input.action)
        })
        .command::<Credential, _, _>(move |input: CredentialInput| {
            credential.credential(input.conversation_id, input.action)
        })
        .command::<TrustCertificate, _, _>(move |input: TrustCertificateInput| {
            trust_certificate.trust_certificate(
                input.conversation_id,
                input.session_id,
                input.hostname,
                input.fingerprint,
            )
        })
        .command::<SavedCertificates, _, _>(move |()| saved_certificates.saved_certificates())
        .command::<SaveCertificate, _, _>(move |input: SavedCertificateInput| {
            save_certificate.save_certificate(input.hostname, input.fingerprint)
        })
        .command::<ForgetCertificate, _, _>(move |input: SavedCertificateInput| {
            forget_certificate.forget_certificate(input.hostname, input.fingerprint)
        })
        .channel::<Follow, _, _>(move |input: ConversationInput| {
            std::future::ready(Ok::<_, BrowserError>(follow.watch(input.conversation_id)))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(rect: BrowserRect, scale: f64, clip: Vec<BrowserRect>) -> PlaceInput {
        PlaceInput {
            conversation_id: ConversationId::parse("0199e0f2-0000-7000-8000-000000000004").unwrap(),
            placement: BrowserPlacement {
                tab_id: "tab-1".to_owned(),
                rect,
                scale,
                visible: true,
                clip,
                cursor_accent: None,
            },
        }
    }

    const RECT: BrowserRect = BrowserRect {
        x: 10.0,
        y: 20.0,
        width: 640.0,
        height: 480.0,
    };

    #[test]
    fn a_placement_accepts_a_measured_rectangle() {
        placement(RECT, 1.5, vec![RECT]).validate().unwrap();
    }

    #[test]
    fn a_placement_accepts_an_optional_cursor_accent_as_three_bytes() {
        let mut input = serde_json::json!({
            "conversationId": "0199e0f2-0000-7000-8000-000000000004",
            "placement": { "tabId": "tab-1", "rect": RECT, "scale": 1.0, "visible": true, "clip": [] }
        });
        let old: PlaceInput = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(old.placement.cursor_accent, None);
        input["placement"]["cursorAccent"] = serde_json::json!([0, 128, 255]);
        let colored: PlaceInput = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(colored.placement.cursor_accent, Some([0, 128, 255]));
        for invalid in [
            serde_json::json!([0, 128]),
            serde_json::json!([0, 128, 255, 255]),
            serde_json::json!([0, 128, 256]),
        ] {
            input["placement"]["cursorAccent"] = invalid;
            serde_json::from_value::<PlaceInput>(input.clone()).unwrap_err();
        }
    }

    #[test]
    fn a_placement_refuses_values_that_are_not_finite_or_too_large() {
        let infinite = BrowserRect {
            width: f64::INFINITY,
            ..RECT
        };
        let negative = BrowserRect {
            height: -1.0,
            ..RECT
        };
        placement(infinite, 1.0, Vec::new()).validate().unwrap_err();
        placement(negative, 1.0, Vec::new()).validate().unwrap_err();
        placement(RECT, f64::NAN, Vec::new())
            .validate()
            .unwrap_err();
        placement(RECT, 20.0, Vec::new()).validate().unwrap_err();
        placement(RECT, 1.0, vec![RECT; MAX_CLIPS + 1])
            .validate()
            .unwrap_err();
    }

    #[test]
    fn a_certificate_exception_requires_bounded_input_for_the_current_session() {
        let mut input = serde_json::json!({
            "conversationId": "0199e0f2-0000-7000-8000-000000000004",
            "sessionId": "0199e0f2-0000-7000-8000-000000000005",
            "hostname": "::1",
            "fingerprint": "AA".repeat(32),
        });
        let parsed: TrustCertificateInput = serde_json::from_value(input.clone()).unwrap();
        parsed.validate().unwrap();

        for (field, oversized) in [("sessionId", 129), ("hostname", 1025), ("fingerprint", 129)] {
            let mut invalid = input.clone();
            invalid[field] = serde_json::json!("a".repeat(oversized));
            serde_json::from_value::<TrustCertificateInput>(invalid)
                .unwrap()
                .validate()
                .unwrap_err();
        }

        input.as_object_mut().unwrap().remove("sessionId");
        serde_json::from_value::<TrustCertificateInput>(input).unwrap_err();
    }
}
