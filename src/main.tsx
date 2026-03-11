import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import "./index.css";
import { AppShell } from "./components/AppShell";
import { AccountsPage } from "./pages/AccountsPage";
import { TransactionsPage } from "./pages/TransactionsPage";
import { ReceiptsPage } from "./pages/ReceiptsPage";
import { TaxPage } from "./pages/TaxPage";
import { InvoicesPage } from "./pages/InvoicesPage";
import { ContactsPage } from "./pages/ContactsPage";
import { SettingsPage } from "./pages/SettingsPage";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <BrowserRouter>
      <Routes>
        <Route element={<AppShell />}>
          <Route path="/accounts" element={<AccountsPage />} />
          <Route path="/transactions" element={<TransactionsPage />} />
          <Route path="/receipts" element={<ReceiptsPage />} />
          <Route path="/tax" element={<TaxPage />} />
          <Route path="/invoices" element={<InvoicesPage />} />
          <Route path="/contacts" element={<ContactsPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="*" element={<Navigate to="/accounts" replace />} />
        </Route>
      </Routes>
    </BrowserRouter>
  </StrictMode>
);
