import { FormEvent, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [name, setName] = useState("");
  const [message, setMessage] = useState(
    "本地优先地观察 Codex 与 Claude Code 的 Token 使用情况。",
  );

  async function submitGreeting(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    const trimmedName = name.trim();
    if (!trimmedName) {
      setMessage("请输入名称后再测试前后端通信。");
      return;
    }

    try {
      const greeting = await invoke<string>("greet", { name: trimmedName });
      setMessage(greeting);
    } catch {
      setMessage("当前运行在浏览器预览中；请通过 Tauri 启动以测试 IPC。");
    }
  }

  return (
    <main className="app-shell">
      <section className="hero" aria-labelledby="app-title">
        <p className="eyebrow">AI coding token observatory</p>
        <h1 id="app-title">TokenBuddy</h1>
        <p className="summary">{message}</p>

        <form className="ipc-check" onSubmit={submitGreeting}>
          <label htmlFor="name">IPC 连通性测试</label>
          <div className="form-row">
            <input
              id="name"
              name="name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              placeholder="你的名字"
              autoComplete="name"
            />
            <button type="submit">发送</button>
          </div>
        </form>
      </section>
    </main>
  );
}

export default App;
