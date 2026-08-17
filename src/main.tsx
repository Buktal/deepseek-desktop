import React from "react"
import ReactDOM from "react-dom/client"
import App from "@/app/App"
// PROTOTYPE(#30):?prototype=1 走壳 UI 原型(分支 prototype/shell-ui,throwaway)
import ShellPrototype from "@/prototype/ShellPrototype"
import "@/i18n"
import "./index.css"

const isPrototype = new URLSearchParams(window.location.search).has("prototype")

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {isPrototype ? <ShellPrototype /> : <App />}
  </React.StrictMode>,
)
