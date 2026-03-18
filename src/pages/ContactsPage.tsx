import { useEffect, useState, useMemo } from "react";
import {
  getContacts,
  createContact,
  updateContact,
  type ContactRecord,
  type ContactInput,
} from "../lib/api";
import { TableSkeleton } from "../components/Skeleton";
import { useToast } from "../components/Toast";

export function ContactsPage() {
  const [contacts, setContacts] = useState<ContactRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [editing, setEditing] = useState<ContactRecord | null>(null);
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<string>("");
  const { toast } = useToast();

  function refresh() {
    setLoading(true);
    getContacts()
      .then(setContacts)
      .catch((e) => toast("error", String(e)))
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    refresh();
  }, []);

  const filtered = useMemo(() => {
    let result = contacts;
    if (search) {
      const q = search.toLowerCase();
      result = result.filter(
        (c) =>
          c.name.toLowerCase().includes(q) ||
          (c.email && c.email.toLowerCase().includes(q)) ||
          (c.phone && c.phone.includes(q))
      );
    }
    if (typeFilter) {
      result = result.filter((c) => c.contact_type === typeFilter);
    }
    return result;
  }, [contacts, search, typeFilter]);

  if (loading && contacts.length === 0) {
    return (
      <div>
        <h2 className="text-xl font-semibold mb-4">Contacts</h2>
        <TableSkeleton rows={5} cols={4} />
      </div>
    );
  }

  return (
    <div>
      <div className="flex items-center justify-between mb-4 flex-wrap gap-2">
        <h2 className="text-xl font-semibold">Contacts</h2>
        <button
          onClick={() => {
            setEditing(null);
            setShowForm(!showForm);
          }}
          className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors"
        >
          {showForm && !editing ? "Cancel" : "New Contact"}
        </button>
      </div>

      {(showForm || editing) && (
        <ContactForm
          initial={editing}
          onSaved={() => {
            setShowForm(false);
            setEditing(null);
            refresh();
            toast("success", editing ? "Contact updated" : "Contact created");
          }}
          onCancel={() => {
            setShowForm(false);
            setEditing(null);
          }}
          onError={(msg) => toast("error", msg)}
        />
      )}

      {/* Filters */}
      <div className="flex gap-3 mb-4">
        <input
          type="text"
          placeholder="Search contacts..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary flex-1 min-w-[200px]"
        />
        <select
          value={typeFilter}
          onChange={(e) => setTypeFilter(e.target.value)}
          className="px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
        >
          <option value="">All types</option>
          <option value="Client">Client</option>
          <option value="Vendor">Vendor</option>
          <option value="Contractor">Contractor</option>
        </select>
      </div>

      {filtered.length === 0 ? (
        <p className="text-text-muted text-sm">
          {contacts.length === 0 ? "No contacts yet. Add one to get started." : "No contacts match your filters."}
        </p>
      ) : (
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-text-muted border-b border-border">
                <th className="py-2 pr-4">Name</th>
                <th className="py-2 pr-4">Type</th>
                <th className="py-2 pr-4">Email</th>
                <th className="py-2 pr-4">Phone</th>
                <th className="py-2 pr-4 w-16"></th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((c) => (
                <tr key={c.id} className="border-b border-border/50">
                  <td className="py-2 pr-4 font-medium">{c.name}</td>
                  <td className="py-2 pr-4">
                    <span className="px-2 py-0.5 rounded-full text-xs bg-gray-100">
                      {c.contact_type}
                      {c.is_contractor && " (1099)"}
                    </span>
                  </td>
                  <td className="py-2 pr-4 text-text-muted">{c.email || "\u2014"}</td>
                  <td className="py-2 pr-4 text-text-muted">{c.phone || "\u2014"}</td>
                  <td className="py-2 pr-4">
                    <button
                      onClick={() => {
                        setEditing(c);
                        setShowForm(false);
                      }}
                      className="text-xs text-primary hover:underline"
                    >
                      Edit
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

function ContactForm({
  initial,
  onSaved,
  onCancel,
  onError,
}: {
  initial: ContactRecord | null;
  onSaved: () => void;
  onCancel: () => void;
  onError: (msg: string) => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [email, setEmail] = useState(initial?.email ?? "");
  const [phone, setPhone] = useState(initial?.phone ?? "");
  const [address, setAddress] = useState(initial?.address ?? "");
  const [contactType, setContactType] = useState(initial?.contact_type ?? "Client");
  const [isContractor, setIsContractor] = useState(initial?.is_contractor ?? false);
  const [taxId, setTaxId] = useState(initial?.tax_id ?? "");
  const [notes, setNotes] = useState(initial?.notes ?? "");
  const [submitting, setSubmitting] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) {
      onError("Name is required");
      return;
    }
    setSubmitting(true);
    try {
      const data: ContactInput = {
        name: name.trim(),
        email: email.trim() || undefined,
        phone: phone.trim() || undefined,
        address: address.trim() || undefined,
        contact_type: contactType,
        is_contractor: isContractor,
        tax_id: taxId.trim() || undefined,
        notes: notes.trim() || undefined,
      };
      if (initial) {
        await updateContact({ id: initial.id, ...data });
      } else {
        await createContact(data);
      }
      onSaved();
    } catch (err) {
      onError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <form onSubmit={handleSubmit} className="bg-surface rounded-lg border border-border p-4 space-y-4 mb-4">
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        <div>
          <label className="block text-xs text-text-muted mb-1">Name *</label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            required
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div>
          <label className="block text-xs text-text-muted mb-1">Email</label>
          <input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div>
          <label className="block text-xs text-text-muted mb-1">Phone</label>
          <input
            type="tel"
            value={phone}
            onChange={(e) => setPhone(e.target.value)}
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div>
          <label className="block text-xs text-text-muted mb-1">Type</label>
          <select
            value={contactType}
            onChange={(e) => setContactType(e.target.value)}
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          >
            <option value="Client">Client</option>
            <option value="Vendor">Vendor</option>
            <option value="Contractor">Contractor</option>
          </select>
        </div>
        <div className="md:col-span-2">
          <label className="block text-xs text-text-muted mb-1">Address</label>
          <input
            type="text"
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div>
          <label className="block text-xs text-text-muted mb-1">Tax ID</label>
          <input
            type="text"
            value={taxId}
            onChange={(e) => setTaxId(e.target.value)}
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary"
          />
        </div>
        <div className="flex items-end">
          <label className="flex items-center gap-2 text-sm pb-1">
            <input
              type="checkbox"
              checked={isContractor}
              onChange={(e) => setIsContractor(e.target.checked)}
              className="rounded"
            />
            1099 Contractor
          </label>
        </div>
        <div className="md:col-span-2">
          <label className="block text-xs text-text-muted mb-1">Notes</label>
          <textarea
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            rows={2}
            className="w-full px-3 py-1.5 text-sm border border-border rounded-md bg-bg focus:outline-none focus:border-primary resize-y"
          />
        </div>
      </div>
      <div className="flex gap-2">
        <button
          type="submit"
          disabled={submitting}
          className="px-4 py-2 text-sm font-medium bg-primary text-white rounded-md hover:bg-primary-hover transition-colors disabled:opacity-50"
        >
          {submitting ? "Saving..." : initial ? "Update Contact" : "Create Contact"}
        </button>
        <button
          type="button"
          onClick={onCancel}
          className="px-4 py-2 text-sm font-medium border border-border rounded-md hover:bg-bg transition-colors"
        >
          Cancel
        </button>
      </div>
    </form>
  );
}
