#!/usr/bin/env node
// Drift check between the hand-written frontend contract (src/dto/models.ts)
// and the typeshare projection of the Rust types (src/dto/generated.ts).
//
// The two files are deliberately not identical: models.ts encodes the
// practical contract (fields the backend always populates are required, not
// optional), renames types that collide with browser globals, and inlines
// trivial enums. This check therefore compares NAMES — interface fields and
// enum/union values — and ignores optionality. Anything not explained by the
// allowlists below is drift and fails the check.
//
// Run via `npm run drift-check` (regenerates generated.ts first).

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { checkTaggedAnswers, TAGGED_ANSWERS } from "./tagged-answer-drift.mjs";

const dto = join(dirname(fileURLToPath(import.meta.url)), "..", "src", "dto");
const generated = readFileSync(join(dto, "generated.ts"), "utf8");
const models = readFileSync(join(dto, "models.ts"), "utf8");
checkTaggedAnswers(readFileSync(join(dto, "../../../../nod-proto/src/answer.rs"), "utf8"), models);

// Rust name -> models.ts name. Renames must keep the same field set.
const RENAMES = {
  Request: "NodRequest", // `Request` collides with the Fetch API global
  CardField: "RequestField", // named for what they are in the request card UI
  CardLink: "RequestLink",
};

// Rust types models.ts deliberately has no counterpart for, with reasons.
const OMITTED_TYPES = {
  // The frontend never *submits* signatures — signing happens in the Rust
  // backend (nod-client-core); the frontend only reads DecisionSignatureRecord.
  DecisionSignature: "signing lives in the Rust backend",
};

// Per-type field omissions models.ts makes on purpose. Currently none — keep
// the mechanism so a future deliberate omission is documented, not silent.
const OMITTED_FIELDS = {};

function parseInterfaces(source) {
  const interfaces = new Map();
  // [^}]* truncates at the first `}`, so inline object field types would cut
  // the body short — neither file uses them; keep it that way.
  const re = /export interface (\w+) \{([^}]*)\}/g;
  for (const match of source.matchAll(re)) {
    const fields = new Set(
      [...match[2].matchAll(/^\s*(\w+)\??:/gm)].map((m) => m[1]),
    );
    interfaces.set(match[1], fields);
  }
  return interfaces;
}

function parseEnumValues(source) {
  const enums = new Map();
  // typeshare style: export enum X { A = "a", ... }
  for (const match of source.matchAll(/export enum (\w+) \{([^}]*)\}/g)) {
    const values = new Set(
      [...match[2].matchAll(/= "([^"]+)"/g)].map((m) => m[1]),
    );
    enums.set(match[1], values);
  }
  // models.ts style: export type X = | "a" | "b";
  for (const match of source.matchAll(/export type (\w+) =([^;]*);/g)) {
    const values = new Set(
      [...match[2].matchAll(/"([^"]+)"/g)].map((m) => m[1]),
    );
    if (values.size > 0) enums.set(match[1], values);
  }
  return enums;
}

const genInterfaces = parseInterfaces(generated);
const modelInterfaces = parseInterfaces(models);
const genEnums = parseEnumValues(generated);
const modelEnums = parseEnumValues(models);

const problems = [];

for (const [genName, genFields] of genInterfaces) {
  if (genName in OMITTED_TYPES) continue;
  const modelName = RENAMES[genName] ?? genName;
  const modelFields = modelInterfaces.get(modelName);
  if (!modelFields) {
    problems.push(`interface ${genName}: missing from models.ts (expected as ${modelName})`);
    continue;
  }
  const allowed = new Set(OMITTED_FIELDS[genName] ?? []);
  for (const field of genFields) {
    if (!modelFields.has(field) && !allowed.has(field)) {
      problems.push(`interface ${modelName}: field "${field}" exists in Rust but not in models.ts`);
    }
  }
  for (const field of modelFields) {
    if (!genFields.has(field)) {
      problems.push(`interface ${modelName}: field "${field}" exists in models.ts but not in Rust`);
    }
  }
}

// Types that exist only in models.ts are drift too (stale after a Rust
// rename/removal). Renamed targets and models-only unions backing a generated
// enum are accounted for.
const expectedModelTypes = new Set(
  [...genInterfaces.keys(), ...genEnums.keys()]
    .filter((name) => !(name in OMITTED_TYPES))
    .map((name) => RENAMES[name] ?? name),
);
// These two unions passed the direct closed-grammar comparison above, not an exemption.
for (const name of TAGGED_ANSWERS) expectedModelTypes.add(name);
for (const name of [...modelInterfaces.keys(), ...modelEnums.keys()]) {
  if (!expectedModelTypes.has(name)) {
    problems.push(`type ${name}: exists in models.ts but has no Rust counterpart`);
  }
}

for (const [genName, genValues] of genEnums) {
  if (genName in OMITTED_TYPES) continue;
  const modelName = RENAMES[genName] ?? genName;
  const modelValues = modelEnums.get(modelName);
  if (!modelValues) {
    problems.push(`enum ${genName}: missing from models.ts (expected as ${modelName})`);
    continue;
  }
  for (const value of genValues) {
    if (!modelValues.has(value)) {
      problems.push(`enum ${modelName}: value "${value}" exists in Rust but not in models.ts`);
    }
  }
  for (const value of modelValues) {
    if (!genValues.has(value)) {
      problems.push(`enum ${modelName}: value "${value}" exists in models.ts but not in Rust`);
    }
  }
}

if (problems.length > 0) {
  console.error("Type drift between src/dto/models.ts and the Rust #[typeshare] types:\n");
  for (const problem of problems) console.error(`  - ${problem}`);
  console.error(
    "\nFix models.ts (or, for a deliberate divergence, document it in the allowlists in scripts/check-drift.mjs).",
  );
  process.exit(1);
}

console.log(
  `drift-check: ${genInterfaces.size} interfaces and ${genEnums.size} enums agree with models.ts`,
);
