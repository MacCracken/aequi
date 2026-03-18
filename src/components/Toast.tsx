import { useState, useCallback, useEffect, useRef, createContext, useContext } from "react";

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
  const timersRef = useRef<Map<number, ReturnType<typeof setTimeout>>>(new Map());

  const dismiss = useCallback((id: number) => {
    setMessages((prev) => prev.filter((m) => m.id !== id));
    const timer = timersRef.current.get(id);
    if (timer) {
      clearTimeout(timer);
      timersRef.current.delete(id);
    }
  }, []);

  const toast = useCallback((type: ToastMessage["type"], text: string) => {
    const id = ++nextId;
    setMessages((prev) => [...prev, { id, type, text }]);
    const timer = setTimeout(() => {
      setMessages((prev) => prev.filter((m) => m.id !== id));
      timersRef.current.delete(id);
    }, 4000);
    timersRef.current.set(id, timer);
  }, []);

  // Clean up all timers on unmount
  useEffect(() => {
    const timers = timersRef.current;
    return () => {
      timers.forEach((timer) => clearTimeout(timer));
      timers.clear();
    };
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
