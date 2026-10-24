// typeshare 1.13.4 cannot project internally tagged struct variants. Compare
// exactly this closed Rust/TS grammar instead; unfamiliar syntax is an error.
export const TAGGED_ANSWERS = ["AnswerValue", "AnswerField"];

function fail(message) { throw new Error(`tagged answer drift: ${message}`); }

function fields(body, separator, rust) {
  const result = {};
  const pieces = body.split(separator).map((part) => part.trim()).filter(Boolean);
  for (const piece of pieces) {
    const match = /^(\w+):\s*(.+)$/.exec(piece);
    if (!match || Object.hasOwn(result, match[1])) fail(`unsupported/duplicate field: ${piece}`);
    const [, name, raw] = match;
    const type = rust ? { String: "string", AnswerValue: "AnswerValue", "Vec<AnswerChoice>": "AnswerChoice[]" }[raw.trim()] : raw.trim();
    if (!["string", "AnswerValue", "AnswerChoice[]"].includes(type)) fail(`unsupported type: ${raw}`);
    result[name] = type;
  }
  return result;
}

function rustVariants(source, name) {
  const declaration = new RegExp(`#\\[serde\\(tag = "kind", rename_all = "snake_case", deny_unknown_fields\\)\\]\\s*pub enum ${name} \\{([\\s\\S]*?)^\\}`, "m").exec(source);
  if (!declaration) fail(`missing/unsupported Rust declaration ${name}`);
  let body = declaration[1].trim();
  const variants = {};
  while (body) {
    const match = /^(\w+)\s*\{([^{}]*)\}\s*,?\s*/.exec(body);
    if (!match) fail(`unsupported Rust variant in ${name}: ${body}`);
    const kind = match[1].replace(/[A-Z]/g, (char, index) => `${index ? "_" : ""}${char.toLowerCase()}`);
    if (Object.hasOwn(variants, kind)) fail(`duplicate kind ${kind}`);
    variants[kind] = fields(match[2], ",", true);
    body = body.slice(match[0].length);
  }
  return variants;
}

function tsVariants(source, name) {
  const start = new RegExp(`export type ${name} =`).exec(source);
  if (!start) fail(`missing TS union ${name}`);
  let body = source.slice(start.index + start[0].length).trimStart();
  const variants = {};
  while (true) {
    const match = /^\|\s*\{\s*kind:\s*"([a-z_]+)";([^{}]*)\}\s*/.exec(body);
    if (!match) fail(`unsupported TS variant in ${name}: ${body.slice(0, 80)}`);
    if (Object.hasOwn(variants, match[1])) fail(`duplicate kind ${match[1]}`);
    variants[match[1]] = fields(match[2], ";", false);
    body = body.slice(match[0].length);
    if (body.startsWith(";")) break;
    if (!body.startsWith("|")) fail(`unsupported TS union tail ${name}`);
  }
  return variants;
}

function canonical(value) {
  return JSON.stringify(Object.fromEntries(Object.entries(value).sort().map(([kind, fields]) => [kind, Object.fromEntries(Object.entries(fields).sort())])));
}

export function checkTaggedAnswers(rust, models) {
  for (const name of TAGGED_ANSWERS) {
    if (canonical(rustVariants(rust, name)) !== canonical(tsVariants(models, name))) fail(`${name}: kinds, required fields or types differ`);
  }
}
