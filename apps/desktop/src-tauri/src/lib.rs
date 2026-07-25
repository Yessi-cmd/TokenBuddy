#[tauri::command]
fn greet(name: &str) -> String {
    format!("你好，{name}！TokenBuddy 的前后端通信正常。")
}

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("failed to run TokenBuddy");
}

#[cfg(test)]
mod tests {
    use super::greet;

    #[test]
    fn greeting_mentions_the_name() {
        assert_eq!(greet("Codex"), "你好，Codex！TokenBuddy 的前后端通信正常。");
    }
}
