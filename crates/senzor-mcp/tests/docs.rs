//! The documentation's tool reference lists exactly the tools the server has.

mod support;

use std::collections::BTreeSet;
use std::path::Path;

use support::{Agent, temp_dir};

/// Tool names from the `` ### `name` `` headings of the reference page.
fn documented_tools() -> BTreeSet<String> {
    let page = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/content/docs/reference/mcp-tools.mdx");
    std::fs::read_to_string(&page)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", page.display()))
        .lines()
        .filter_map(|line| line.strip_prefix("### `")?.strip_suffix('`'))
        .map(String::from)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tool_reference_lists_every_tool_and_nothing_else() {
    let agent = Agent::open(None, false, 1, temp_dir("docs")).await;
    let served: BTreeSet<String> = agent
        .client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    let documented = documented_tools();
    let undocumented: Vec<_> = served.difference(&documented).collect();
    let unknown: Vec<_> = documented.difference(&served).collect();
    assert!(
        undocumented.is_empty() && unknown.is_empty(),
        "not in reference/mcp-tools.mdx: {undocumented:?}; documented but not served: {unknown:?}"
    );
    agent.finish().await;
}
