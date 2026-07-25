import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { open, realpath, stat } from "node:fs/promises";

export async function inspectPortableExecutable(
  filePath,
  { minBytes = 128 * 1024 } = {},
) {
  const resolvedPath = await realpath(filePath);
  const fileStats = await stat(resolvedPath);

  if (!fileStats.isFile()) {
    throw new Error(`PE doğrulaması bir dosya bekliyordu: ${filePath}`);
  }
  if (fileStats.size < minBytes) {
    throw new Error(
      `Dosya beklenenden küçük (${fileStats.size} bayt): ${filePath}`,
    );
  }

  const handle = await open(resolvedPath, "r");
  try {
    const dosHeader = Buffer.alloc(64);
    const dosRead = await handle.read(dosHeader, 0, dosHeader.length, 0);
    if (
      dosRead.bytesRead !== dosHeader.length ||
      dosHeader[0] !== 0x4d ||
      dosHeader[1] !== 0x5a
    ) {
      throw new Error(`Geçerli MZ başlığı bulunamadı: ${filePath}`);
    }

    const peOffset = dosHeader.readUInt32LE(0x3c);
    if (peOffset < 0x40 || peOffset > fileStats.size - 4) {
      throw new Error(`PE başlık konumu geçersiz: ${filePath}`);
    }

    const signature = Buffer.alloc(4);
    const peRead = await handle.read(signature, 0, signature.length, peOffset);
    if (
      peRead.bytesRead !== signature.length ||
      signature[0] !== 0x50 ||
      signature[1] !== 0x45 ||
      signature[2] !== 0x00 ||
      signature[3] !== 0x00
    ) {
      throw new Error(`Geçerli PE imzası bulunamadı: ${filePath}`);
    }
  } finally {
    await handle.close();
  }

  const hash = createHash("sha256");
  for await (const chunk of createReadStream(resolvedPath)) {
    hash.update(chunk);
  }

  return {
    path: resolvedPath,
    size: fileStats.size,
    sha256: hash.digest("hex"),
  };
}
