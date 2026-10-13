import { useRef } from "react";

interface CameraCaptureProps {
  onCapture: (file: File) => void;
  /** Disables the button (e.g. while a previous capture is being processed). */
  busy?: boolean;
  className?: string;
}

/**
 * Mobile receipt capture using the WebView's native file input API.
 * On iOS/Android, `capture="environment"` opens the rear camera directly.
 * On desktop, it falls back to a standard file picker.
 */
export function CameraCapture({ onCapture, busy = false, className }: CameraCaptureProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  function handleChange(e: React.ChangeEvent<HTMLInputElement>) {
    const file = e.target.files?.[0];
    // Reset so the same file can be re-selected
    e.target.value = "";
    if (file) onCapture(file);
  }

  return (
    <>
      <button
        type="button"
        onClick={() => inputRef.current?.click()}
        disabled={busy}
        className={`inline-flex items-center gap-2 px-4 py-2 rounded-md text-sm font-medium bg-primary text-white hover:bg-primary-hover transition-colors disabled:opacity-50 ${className ?? ""}`}
      >
        <svg className="w-4 h-4" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2} aria-hidden="true">
          <path strokeLinecap="round" strokeLinejoin="round" d="M3 9a2 2 0 012-2h.93a2 2 0 001.664-.89l.812-1.22A2 2 0 0110.07 4h3.86a2 2 0 011.664.89l.812 1.22A2 2 0 0018.07 7H19a2 2 0 012 2v9a2 2 0 01-2 2H5a2 2 0 01-2-2V9z" />
          <path strokeLinecap="round" strokeLinejoin="round" d="M15 13a3 3 0 11-6 0 3 3 0 016 0z" />
        </svg>
        {busy ? "Processing..." : "Capture Receipt"}
      </button>
      {/* Kept outside the <button>: interactive content may not nest inside it. */}
      <input
        ref={inputRef}
        type="file"
        accept="image/*"
        capture="environment"
        onChange={handleChange}
        className="hidden"
        tabIndex={-1}
        aria-hidden="true"
      />
    </>
  );
}
