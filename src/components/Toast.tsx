import { useState, useCallback, createContext, useContext } from "react";

interface ToastMessage {
  id: number;
  type: "success" | "error" | "info";
  text: string;
}

interface ToastContextType {
  toast: (type: ToastMessage["type"], text: string) => void;
}

const ToastContext = createContext<ToastContextType>({ toast: () => {} });

export function useToast() {
  return useContext(ToastContext);
}

let nextId = 0;

export function ToastProvider({ children }: { children: React.ReactNode }) {
  const [messages, setMessages] = useState<ToastMessage[]>([]);

  const toast = useCallback((type: ToastMessage["type"], text: string) => {
    const id = ++nextId;
    setMessages((prev) => [...prev, { id, type, text }]);
    setTimeout(() => {
      setMessages((prev) => prev.filter((m) => m.id !== id));
    }, 4000);
  }, []);

  const dismiss = useCallback((id: number) => {
    setMessages((prev) => prev.filter((m) => m.id !== id));
  }, []);

  return (
    <ToastContext.Provider value={{ toast }}>
      {children}
      <div className="fixed top-4 right-4 z-[100] flex flex-col gap-2 max-w-sm">
        {messages.map((m) => (
          <div
            key={m.id}
            role="alert"
            onClick={() => dismiss(m.id)}
            className={`px-4 py-3 rounded-lg shadow-lg text-sm font-medium cursor-pointer transition-all animate-slide-in ${
              m.type === "error"
                ? "bg-danger text-white"
                : m.type === "success"
                  ? "bg-success text-white"
                  : "bg-primary text-white"
            }`}
          >
            {m.text}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
