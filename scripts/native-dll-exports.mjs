// Read the export name table of a PE (Windows DLL) file. Used by the native
// broker build scripts to pin each DLL's C ABI without external tools.
import { readFileSync } from "node:fs";

function rvaToOffset(sections, rva) {
  for (const section of sections) {
    const size = Math.max(section.virtualSize, section.rawSize);
    if (rva >= section.virtualAddress && rva < section.virtualAddress + size) {
      return rva - section.virtualAddress + section.rawPointer;
    }
  }
  throw new Error(`RVA 0x${rva.toString(16)} is outside every section`);
}

function readCString(buffer, offset) {
  let end = offset;
  while (end < buffer.length && buffer[end] !== 0) end += 1;
  return buffer.toString("latin1", offset, end);
}

/** Sorted exported symbol names of `dllPath`. */
export function readDllExports(dllPath) {
  const buffer = readFileSync(dllPath);
  if (buffer.toString("latin1", 0, 2) !== "MZ") throw new Error(`${dllPath} is not a PE file`);
  const pe = buffer.readUInt32LE(0x3c);
  if (buffer.toString("latin1", pe, pe + 4) !== "PE\0\0") throw new Error(`${dllPath} has no PE header`);
  const coff = pe + 4;
  const sectionCount = buffer.readUInt16LE(coff + 2);
  const optionalSize = buffer.readUInt16LE(coff + 16);
  const optional = coff + 20;
  const magic = buffer.readUInt16LE(optional);
  const dataDirectories = optional + (magic === 0x20b ? 112 : 96);
  const exportRva = buffer.readUInt32LE(dataDirectories);
  const sections = [];
  for (let index = 0; index < sectionCount; index += 1) {
    const header = optional + optionalSize + index * 40;
    sections.push({
      virtualSize: buffer.readUInt32LE(header + 8),
      virtualAddress: buffer.readUInt32LE(header + 12),
      rawSize: buffer.readUInt32LE(header + 16),
      rawPointer: buffer.readUInt32LE(header + 20),
    });
  }
  if (exportRva === 0) return [];
  const directory = rvaToOffset(sections, exportRva);
  const nameCount = buffer.readUInt32LE(directory + 24);
  const namesRva = buffer.readUInt32LE(directory + 32);
  const names = [];
  const namesOffset = rvaToOffset(sections, namesRva);
  for (let index = 0; index < nameCount; index += 1) {
    const nameRva = buffer.readUInt32LE(namesOffset + index * 4);
    names.push(readCString(buffer, rvaToOffset(sections, nameRva)));
  }
  return names.sort();
}

/** Lines of a committed export list (blank lines and `#` comments ignored). */
export function readExportList(listPath) {
  return readFileSync(listPath, "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .sort();
}
