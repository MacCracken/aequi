import { useEffect, useState } from "react";
import { getContacts, type ContactRecord } from "../lib/api";

export function ContactsPage() {
  const [contacts, setContacts] = useState<ContactRecord[]>([]);

  useEffect(() => {
    getContacts().then(setContacts).catch(console.error);
  }, []);

  return (
    <div>
      <h2 className="text-xl font-semibold mb-4">Contacts</h2>

      {contacts.length === 0 ? (
        <p className="text-text-muted text-sm">No contacts yet.</p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-text-muted border-b border-border">
                <th className="py-2 pr-4">Name</th>
                <th className="py-2 pr-4">Type</th>
                <th className="py-2 pr-4">Email</th>
                <th className="py-2 pr-4">Phone</th>
              </tr>
            </thead>
            <tbody>
              {contacts.map((c) => (
                <tr key={c.id} className="border-b border-border/50">
                  <td className="py-2 pr-4 font-medium">{c.name}</td>
                  <td className="py-2 pr-4">
                    <span className="px-2 py-0.5 rounded-full text-xs bg-gray-100">
                      {c.contact_type}
                      {c.is_contractor && " (1099)"}
                    </span>
                  </td>
                  <td className="py-2 pr-4 text-text-muted">{c.email || "—"}</td>
                  <td className="py-2 pr-4 text-text-muted">{c.phone || "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
