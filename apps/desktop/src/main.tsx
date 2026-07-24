import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { minosQueryClient } from "./shared/api/queryClient";
import { ThemeProvider } from "./shared/theme/ThemeProvider";
import { ErrorBoundary } from "./shared/ui/ErrorBoundary";
import "./index.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={minosQueryClient}>
      <ThemeProvider>
        {/* Root boundary: catches boot-path crashes so reveal still fires. */}
        <ErrorBoundary label="root">
          <App />
        </ErrorBoundary>
      </ThemeProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
