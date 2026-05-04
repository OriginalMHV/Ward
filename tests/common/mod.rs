use serde_json::json;

#[allow(dead_code)]
pub fn make_repo_json(name: &str, archived: bool) -> serde_json::Value {
    json!({
        "name": name,
        "full_name": format!("test-org/{name}"),
        "archived": archived,
        "default_branch": "main",
        "description": format!("Repo {name}"),
        "visibility": "private",
        "language": "Kotlin"
    })
}
