import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const expectedFileName = "fylura_windows_preview_handler.dll";
const defaultDllPath = path.join(
  repositoryRoot,
  "src-tauri",
  "native",
  "packaged",
  expectedFileName,
);
const dllPath = process.argv[2]
  ? path.resolve(process.argv[2])
  : defaultDllPath;

const disallowedRuntimeImport =
  /^(?:api-ms-win-crt-|vcruntime\d*(?:_\d+)?|msvcp\d*(?:_\d+)?|concrt\d*|ucrtbase)\.dll$/iu;

function fail(message) {
  throw new Error(`[preview-handler-dependencies] ${message}`);
}

function assertRange(buffer, offset, size, label) {
  if (
    !Number.isInteger(offset) ||
    offset < 0 ||
    offset + size > buffer.length
  ) {
    fail(`malformed PE ${label} range at ${offset} (${size} bytes)`);
  }
}

function readUInt16(buffer, offset, label) {
  assertRange(buffer, offset, 2, label);
  return buffer.readUInt16LE(offset);
}

function readUInt32(buffer, offset, label) {
  assertRange(buffer, offset, 4, label);
  return buffer.readUInt32LE(offset);
}

function readAsciiZ(buffer, offset, label) {
  assertRange(buffer, offset, 1, label);
  const end = buffer.indexOf(0, offset);
  if (end < 0) {
    fail(`unterminated PE ${label} string at ${offset}`);
  }
  return buffer.toString("ascii", offset, end);
}

function mapRvaToOffset(rva, sections, label) {
  for (const section of sections) {
    const span = Math.max(section.virtualSize, section.rawSize);
    if (rva >= section.virtualAddress && rva < section.virtualAddress + span) {
      const offset = section.rawPointer + (rva - section.virtualAddress);
      assertRange(bufferForRva, offset, 1, label);
      return offset;
    }
  }
  fail(`PE ${label} RVA 0x${rva.toString(16)} is outside all sections`);
}

// mapRvaToOffset is bound to the inspected buffer for concise bounds checks.
let bufferForRva;

function parsePe(buffer) {
  bufferForRva = buffer;
  assertRange(buffer, 0, 0x40, "DOS header");
  if (buffer.toString("ascii", 0, 2) !== "MZ") {
    fail("file is not an MZ PE image");
  }

  const peOffset = readUInt32(buffer, 0x3c, "PE header pointer");
  assertRange(buffer, peOffset, 24, "COFF header");
  if (buffer.toString("ascii", peOffset, peOffset + 4) !== "PE\0\0") {
    fail("PE signature is missing");
  }

  const coffOffset = peOffset + 4;
  const machine = readUInt16(buffer, coffOffset, "machine");
  const sectionCount = readUInt16(buffer, coffOffset + 2, "section count");
  const optionalSize = readUInt16(
    buffer,
    coffOffset + 16,
    "optional-header size",
  );
  const optionalOffset = coffOffset + 20;
  assertRange(buffer, optionalOffset, optionalSize, "optional header");
  const optionalMagic = readUInt16(
    buffer,
    optionalOffset,
    "optional-header magic",
  );
  const dataDirectoryOffset =
    optionalOffset + (optionalMagic === 0x20b ? 112 : 96);
  if (optionalMagic !== 0x20b) {
    fail(`expected PE32+ optional header, got 0x${optionalMagic.toString(16)}`);
  }
  assertRange(buffer, dataDirectoryOffset, 16, "data directories");

  const sectionOffset = optionalOffset + optionalSize;
  assertRange(buffer, sectionOffset, sectionCount * 40, "section table");
  const sections = [];
  for (let index = 0; index < sectionCount; index += 1) {
    const offset = sectionOffset + index * 40;
    sections.push({
      virtualSize: readUInt32(
        buffer,
        offset + 8,
        `section ${index} virtual size`,
      ),
      virtualAddress: readUInt32(buffer, offset + 12, `section ${index} RVA`),
      rawSize: readUInt32(buffer, offset + 16, `section ${index} raw size`),
      rawPointer: readUInt32(
        buffer,
        offset + 20,
        `section ${index} raw pointer`,
      ),
    });
  }

  const directory = (index, label) => {
    const offset = dataDirectoryOffset + index * 8;
    assertRange(buffer, offset, 8, `${label} directory`);
    return {
      rva: readUInt32(buffer, offset, `${label} RVA`),
      size: readUInt32(buffer, offset + 4, `${label} size`),
    };
  };

  const importDirectory = directory(1, "import");
  const exportDirectory = directory(0, "export");
  const imports = new Set();
  if (importDirectory.rva !== 0 && importDirectory.size !== 0) {
    let descriptorOffset = mapRvaToOffset(
      importDirectory.rva,
      sections,
      "import directory",
    );
    const maxDescriptors = Math.floor(importDirectory.size / 20) + 1;
    for (let index = 0; index < maxDescriptors; index += 1) {
      assertRange(buffer, descriptorOffset, 20, `import descriptor ${index}`);
      const originalFirstThunk = readUInt32(
        buffer,
        descriptorOffset,
        "import thunk",
      );
      const nameRva = readUInt32(
        buffer,
        descriptorOffset + 12,
        "import name RVA",
      );
      const firstThunk = readUInt32(
        buffer,
        descriptorOffset + 16,
        "import address thunk",
      );
      if (originalFirstThunk === 0 && nameRva === 0 && firstThunk === 0) {
        break;
      }
      if (nameRva === 0) {
        fail(`import descriptor ${index} has no DLL name`);
      }
      const nameOffset = mapRvaToOffset(
        nameRva,
        sections,
        `import ${index} name`,
      );
      imports.add(
        readAsciiZ(buffer, nameOffset, `import ${index} name`).toLowerCase(),
      );
      descriptorOffset += 20;
    }
  }

  const exports = new Set();
  if (exportDirectory.rva !== 0 && exportDirectory.size !== 0) {
    const exportOffset = mapRvaToOffset(
      exportDirectory.rva,
      sections,
      "export directory",
    );
    assertRange(buffer, exportOffset, 40, "export directory contents");
    const nameCount = readUInt32(
      buffer,
      exportOffset + 24,
      "export name count",
    );
    const namesRva = readUInt32(buffer, exportOffset + 32, "export names RVA");
    const namesOffset = mapRvaToOffset(namesRva, sections, "export names");
    for (let index = 0; index < nameCount; index += 1) {
      const nameRva = readUInt32(
        buffer,
        namesOffset + index * 4,
        `export name ${index} RVA`,
      );
      const nameOffset = mapRvaToOffset(
        nameRva,
        sections,
        `export name ${index}`,
      );
      exports.add(readAsciiZ(buffer, nameOffset, `export name ${index}`));
    }
  }

  return {
    machine,
    optionalMagic,
    sectionCount,
    imports: [...imports].sort(),
    exports: [...exports].sort(),
  };
}

function verify() {
  if (path.basename(dllPath).toLowerCase() !== expectedFileName) {
    fail(`unexpected DLL name: ${dllPath}`);
  }
  if (!fs.existsSync(dllPath)) {
    fail(`expected packaged DLL is missing: ${dllPath}`);
  }
  const stats = fs.statSync(dllPath);
  if (!stats.isFile() || stats.size === 0) {
    fail(`packaged DLL is empty or not a file: ${dllPath}`);
  }

  const inspection = parsePe(fs.readFileSync(dllPath));
  if (inspection.machine !== 0x8664) {
    fail(
      `expected x64 PE machine 0x8664, got 0x${inspection.machine.toString(16)}`,
    );
  }
  for (const exportName of ["DllGetClassObject", "DllCanUnloadNow"]) {
    if (!inspection.exports.includes(exportName)) {
      fail(`expected COM export is missing: ${exportName}`);
    }
  }

  const disallowedImports = inspection.imports.filter((name) =>
    disallowedRuntimeImport.test(name),
  );
  if (disallowedImports.length > 0) {
    fail(
      `disallowed MSVC runtime imports remain: ${disallowedImports.join(", ")}`,
    );
  }

  console.log(
    JSON.stringify(
      {
        status: "PASS",
        dll: dllPath,
        size: stats.size,
        machine: `0x${inspection.machine.toString(16)}`,
        optionalHeader: `0x${inspection.optionalMagic.toString(16)}`,
        imports: inspection.imports,
        exports: inspection.exports.filter((name) =>
          ["DllGetClassObject", "DllCanUnloadNow"].includes(name),
        ),
        disallowedRuntimeImports: [],
      },
      null,
      2,
    ),
  );
}

verify();
