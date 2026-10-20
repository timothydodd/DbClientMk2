import { confirm as tauriConfirm } from "@tauri-apps/plugin-dialog";

/**
 * Reliable confirmation dialog. The webview's native `window.confirm` is not dependable across
 * platforms (WebView2 in particular), so destructive confirmations go through the Tauri dialog
 * plugin; the in-app Modal-based confirm (stores/ui) is preferred for everything styled.
 */
export async function confirmDestructive(message: string, title = "Confirm"): Promise<boolean> {
  try {
    return await tauriConfirm(message, { title, kind: "warning" });
  } catch {
    return window.confirm(message);
  }
}
