#!/usr/bin/env node
// Validate .glb files with the Khronos glTF-Validator (npm `gltf-validator`).
//
//   NODE_PATH=<dir containing node_modules> node tools/validate_gltf.js <file-or-dir>...
//
// External image URIs are resolved relative to each file and validated too.
// Prints per-file counts and an issue-code histogram; exit code 1 on errors.
const fs = require("fs");
const path = require("path");
const validator = require("gltf-validator");

function collect(p, out) {
  if (fs.statSync(p).isDirectory()) {
    for (const e of fs.readdirSync(p).sort()) collect(path.join(p, e), out);
  } else if (p.endsWith(".glb")) out.push(p);
}

(async () => {
  const files = [];
  for (const a of process.argv.slice(2)) collect(a, files);
  const codes = {};
  let errors = 0, warnings = 0;
  for (const f of files) {
    const report = await validator.validateBytes(new Uint8Array(fs.readFileSync(f)), {
      uri: f,
      maxIssues: 0,
      externalResourceFunction: (uri) =>
        new Promise((resolve, reject) => {
          fs.readFile(path.resolve(path.dirname(f), decodeURIComponent(uri)), (err, data) =>
            err ? reject(err.toString()) : resolve(new Uint8Array(data)));
        }),
    });
    const i = report.issues;
    errors += i.numErrors;
    warnings += i.numWarnings;
    for (const m of i.messages) {
      const k = `${["ERROR", "WARNING", "INFO", "HINT"][m.severity]} ${m.code}`;
      codes[k] = (codes[k] || 0) + 1;
    }
    if (files.length <= 5 || i.numErrors > 0) {
      console.log(`${path.basename(f)}: ${i.numErrors} errors, ${i.numWarnings} warnings, ${i.numInfos} infos`);
    }
  }
  console.log(`${files.length} files: ${errors} errors, ${warnings} warnings`);
  for (const [k, n] of Object.entries(codes).sort((a, b) => b[1] - a[1])) console.log(`${String(n).padStart(8)}  ${k}`);
  process.exit(errors > 0 ? 1 : 0);
})();
