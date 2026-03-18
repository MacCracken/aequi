import { writeFile, mkdir, BaseDirectory } from "@tauri-apps/plugin-fs";
import { appDataDir } from "@tauri-apps/api/path";

/**
 * Write a captured camera File to the app's intake directory so the
 * backend receipt pipeline picks it up. Returns the absolute file path.
 */
export async function writeCapturedFile(file: File): Promise<string> {
  const dir = "intake";
  await mkdir(dir, { baseDir: BaseDirectory.AppData, recursive: true });

  const rawExt = (file.name.split(".").pop() ?? "jpg").toLowerCase();
  const allowedExts = ["jpg", "jpeg", "png", "gif", "webp", "tiff", "tif", "bmp", "pdf"];
  const ext = allowedExts.includes(rawExt) ? rawExt : "jpg";
  const name = `capture_${Date.now()}.${ext}`;
  const path = `${dir}/${name}`;

  const buffer = new Uint8Array(await file.arrayBuffer());
  await writeFile(path, buffer, { baseDir: BaseDirectory.AppData });

  const base = await appDataDir();
  return `${base}${path}`;
}
