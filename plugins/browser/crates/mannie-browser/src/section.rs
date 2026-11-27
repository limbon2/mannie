//! The section `browser` of the system prompt: explains Browser site
//! credentials, exact-origin matching and protected password fills for runs
//! offered the browser tools.

use mannie_plugin_sdk::points::sections::{SectionContext, SectionSource};

/// The section's ID.
pub(crate) const SECTION_ID: &str = "browser";

/// The section's place, after Proxmox (350).
pub(crate) const SECTION_POSITION: u32 = 355;

const BROWSER_OBSERVE: &str = "browser_observe";
const BROWSER_INTERACT: &str = "browser_interact";
const MANNIE_INSPECT: &str = "mannie_inspect";
const MANNIE_CONFIGURE: &str = "mannie_configure";

/// Instruction section describing the Browser plugin's credential workflow.
#[derive(Debug)]
pub(crate) struct BrowserSection;

impl SectionSource for BrowserSection {
    fn render(&self, context: &SectionContext) -> Option<String> {
        let offers_browser = context.uses_tools
            && (context.offers_tool(BROWSER_OBSERVE) || context.offers_tool(BROWSER_INTERACT));
        if !offers_browser {
            return None;
        }

        let can_inspect = context.offers_tool(MANNIE_INSPECT);
        let can_configure = context.offers_tool(MANNIE_CONFIGURE);
        Some(render_section(can_inspect, can_configure))
    }
}

fn render_section(can_inspect: bool, can_configure: bool) -> String {
    let mut parts = vec![
        "Browser lets you open and drive websites in a live pane beside this conversation. \
         The owner can save site credentials (passwords or codes, scoped to an exact canonical \
         `scheme://host[:port]` origin) and TLS certificate pins in Settings > Browser."
            .to_owned(),
    ];

    if can_inspect {
        parts.push(
            "Before searching files, logs, or provisioning inventories for a web UI address, port, \
             or password, call `mannie_inspect(topic=\"plugin\", target=\"browser\")` (read-only, \
             no prompt) to list saved site credentials (`origin`, `label`, `reference`, without \
             secret values) and saved certificate pins. When a saved credential matches the site \
             you need, open its exact saved `origin` with `browser_navigate`."
                .to_owned(),
        );
    }

    parts.push(
        "After navigating to a page, call `browser_observe`; its result includes a `credentials` \
         array (`reference`, `origin`, `label`) of saved credentials matching the observed page's \
         exact origin. Password fields reject plain text (`value: {\"kind\":\"text\",\"text\":\"...\"}`). \
         To fill a password or protected field with a saved credential, call `browser_interact` \
         with `action: {\"kind\":\"fill\",\"reference\":\"<element_ref>\",\"value\":{\"kind\":\"credential\",\"reference\":\"<credential_uuid>\"}}` \
         using the 36-character UUID `reference` (never the label). Mannie prompts the owner to \
         approve using that credential for this conversation and enters the secret directly \
         without exposing its value."
            .to_owned(),
    );

    let missing_guidance = if can_configure {
        "If no saved credential exists for the site, do not ask the owner to paste a real \
         password into chat: propose `mannie_configure(target=\"browser\", action=\"add_site_credential\", \
         arguments={\"origin\":\"...\",\"label\":\"...\"})` omitting the secret so the owner \
         enters it on the masked approval card, ask the owner to add one in Settings > Browser, \
         or ask the owner to take over the Browser pane and sign in directly."
    } else {
        "If no saved credential exists for the site, do not ask the owner to paste a real \
         password into chat: ask the owner to add a site credential in Settings > Browser or \
         take over the Browser pane and sign in directly."
    };
    parts.push(missing_guidance.to_owned());

    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::points::sections::{SectionContext, SectionSource};
    use serde_json::json;

    use super::{
        BROWSER_INTERACT, BROWSER_OBSERVE, BrowserSection, MANNIE_CONFIGURE, MANNIE_INSPECT,
    };

    fn context(uses_tools: bool, offered_tools: &[&str]) -> SectionContext {
        serde_json::from_value(json!({
            "agent": "conversation",
            "requester": "owner",
            "usesTools": uses_tools,
            "offersTasks": false,
            "fullAccess": false,
            "offeredTools": offered_tools,
        }))
        .unwrap()
    }

    #[test]
    fn omits_section_without_browser_tools() {
        assert_eq!(
            BrowserSection.render(&context(false, &[BROWSER_OBSERVE])),
            None
        );
        assert_eq!(BrowserSection.render(&context(true, &["read_file"])), None);
    }

    #[test]
    fn renders_credential_workflow_and_adapts_to_self_management_tools() {
        let plain = BrowserSection
            .render(&context(true, &[BROWSER_OBSERVE, BROWSER_INTERACT]))
            .unwrap();
        assert!(plain.contains("browser_observe"));
        assert!(plain.contains(r#""kind":"credential""#));
        assert!(plain.contains("36-character UUID"));
        assert!(!plain.contains(MANNIE_INSPECT));
        assert!(!plain.contains(MANNIE_CONFIGURE));

        let with_self_management = BrowserSection
            .render(&context(
                true,
                &[
                    BROWSER_OBSERVE,
                    BROWSER_INTERACT,
                    MANNIE_INSPECT,
                    MANNIE_CONFIGURE,
                ],
            ))
            .unwrap();
        assert!(
            with_self_management.contains(r#"mannie_inspect(topic="plugin", target="browser")"#)
        );
        assert!(with_self_management.contains("add_site_credential"));
    }
}
