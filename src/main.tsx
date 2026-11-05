import React from "react";
import ReactDOM from "react-dom/client";
import "./styles.css";

const App = React.lazy(() => import("./App"));

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode><React.Suspense fallback={null}><App /></React.Suspense></React.StrictMode>,
);
