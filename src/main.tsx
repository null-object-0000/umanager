import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import "./styles.css";

// 快捷面板不加载主界面、Markdown 渲染器和软件图标等模块。
const isPanel = "__TAURI_INTERNALS__" in window && getCurrentWebviewWindow().label === "clipboard-panel";
const App = React.lazy(isPanel ? () => import("./ClipboardPanel") : () => import("./App"));

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode><React.Suspense fallback={null}><App /></React.Suspense></React.StrictMode>,
);
