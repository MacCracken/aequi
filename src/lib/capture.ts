import { writeFile, mkdir, remove, BaseDirectory } from "@tauri-apps/plugin-fs";
import { appDataDir, join } from "@tauri-apps/api/path";
import { ingestReceipt, type ReceiptOutput } from "./api";

const MAX_FILE_SIZE = 50 * 1024 * 1024; // 50 MB
const ALLOWED_EXTS = ["jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp"];

// Staging folder for camera captures. Deliberately NOT the watched "intake"
// folder: a file written there would be picked up by the watcher as well as
// by the explicit ingest call below, i.e. processed twice.
const CAPTURE_DIR = "captures";

/**
 * Write a captured camera File to the app-data captures directory and run it
 * through the receipt pipeline. The backend deletes the capture after a
 * successful ingest; on failure the file is removed here (best effort).
 */
export async function ingestCapturedFile(file: File): Promise<ReceiptOutput> {
  if (file.size > MAX_FILE_SIZE) {
    throw new Error(`File too large (${(file.size / 1048576).toFixed(1)} MB, max 50 MB)`);
  }

  await mkdir(CAPTURE_DIR, { baseDir: BaseDirectory.AppData, recursive: true });

  const rawExt = (file.name.split(".").pop() ?? "jpg").toLowerCase();
  const ext = ALLOWED_EXTS.includes(rawExt) ? rawExt : "jpg";
  const suffix = Math.random().toString(36).slice(2, 8);
  const name = `capture_${Date.now()}_${suffix}.${ext}`;
  const relPath = `${CAPTURE_DIR}/${name}`;

  const buffer = new Uint8Array(await file.arrayBuffer());
  await writeFile(relPath, buffer, { baseDir: BaseDirectory.AppData });

  try {
    // join() inserts the platform separator; appDataDir() has no trailing one.
    const absPath = await join(await appDataDir(), CAPTURE_DIR, name);
    return await ingestReceipt(absPath);
  } catch (e) {
    await remove(relPath, { baseDir: BaseDirectory.AppData }).catch(() => {});
    throw e;
  }
}
