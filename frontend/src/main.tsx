import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter } from "react-router";
import "./index.css";
import App from "./App";
import PairGate from "./components/PairGate";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserRouter>
      <PairGate>
        <App />
      </PairGate>
    </BrowserRouter>
  </StrictMode>,
);
