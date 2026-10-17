//! Read-only journal viewer using the canonical export tool.
use super::*;
use niles::integration::ui::{
    ActionResult, ActionValue, Button, ButtonStyle, Node, NodeKind, TextBlock,
};
thread_local! { static PAGE: RefCell<Option<String>> = const { RefCell::new(None) }; }
pub fn reset() {
    PAGE.with(|p| *p.borrow_mut() = None);
}
pub fn append(nodes: &mut Vec<Node>) {
    let page = PAGE.with(|p| p.borrow().clone());
    let mut extra = vec![Node {
        kind: NodeKind::Button(Button {
            id: "diagnostics:first".into(),
            label: "Read diagnostic journal".into(),
            style: ButtonStyle::Secondary,
        }),
        children: vec![],
    }];
    if let Some(raw) = page {
        extra.push(Node { kind: NodeKind::Text(TextBlock { value: "Sensitive household timing data. Copy this JSON page for export; restart from the first page if the journal changes.".into(), muted: true }), children: vec![] });
        extra.push(Node {
            kind: NodeKind::Text(TextBlock {
                value: raw.clone(),
                muted: false,
            }),
            children: vec![],
        });
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw)
            && v["has_more"] == true
            && let (Some(page), Some(snapshot)) = (v["page"].as_u64(), v["snapshot"].as_str())
        {
            extra.push(Node {
                kind: NodeKind::Button(Button {
                    id: format!("diagnostics:next:{}:{snapshot}", page + 1),
                    label: "Next diagnostic page".into(),
                    style: ButtonStyle::Secondary,
                }),
                children: vec![],
            });
        }
        extra.push(Node {
            kind: NodeKind::Button(Button {
                id: "diagnostics:hide".into(),
                label: "Hide diagnostic journal".into(),
                style: ButtonStyle::Secondary,
            }),
            children: vec![],
        });
    }
    for node in extra {
        let index = nodes.len() as u32;
        nodes[0].children.push(index);
        nodes.push(node);
    }
}
pub fn action(id: &str, value: ActionValue) -> Result<ActionResult, String> {
    if !matches!(value, ActionValue::Pressed) {
        return Err("Diagnostic controls require a button press".into());
    }
    if id == "diagnostics:hide" {
        reset();
    } else {
        let args = if id == "diagnostics:first" {
            serde_json::json!({"page":0,"per_page":25})
        } else {
            let parts: Vec<_> = id.split(':').collect();
            if parts.len() != 4 || parts[0] != "diagnostics" || parts[1] != "next" {
                return Err("Invalid diagnostic control".into());
            }
            let page: usize = parts[2].parse().map_err(|_| "Invalid diagnostic page")?;
            serde_json::json!({"page":page,"per_page":25,"snapshot":parts[3]})
        };
        // Clear stale output before a failed export; never display it as current.
        reset();
        let raw = Plugin::call_tool("withings_diagnostics_export".into(), args.to_string())?;
        PAGE.with(|p| *p.borrow_mut() = Some(raw));
    }
    refresh_panel();
    Ok(ActionResult {
        toast: None,
        rerender: true,
    })
}
