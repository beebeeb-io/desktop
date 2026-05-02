import { invoke } from "@tauri-apps/api/core";

async function mount() {
  const root = document.getElementById("root");
  if (!root) return;
  let version = "?";
  try {
    version = await invoke<string>("app_version");
  } catch {}
  root.innerHTML = `
    <main style="font-family: Inter, system-ui, sans-serif; padding: 40px;">
      <h1 style="margin: 0 0 8px;">Beebeeb</h1>
      <p style="margin: 0; color: #555;">Desktop scaffold v${version} — frontend will be replaced by the web client.</p>
    </main>
  `;
}

mount();
