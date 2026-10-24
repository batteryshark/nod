// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { checkTaggedAnswers } from "../scripts/tagged-answer-drift.mjs";

const rust = readFileSync(new URL("../../../nod-proto/src/answer.rs", import.meta.url), "utf8");
const models = readFileSync(new URL("./dto/models.ts", import.meta.url), "utf8");

describe("closed tagged answer drift gate", () => {
  it("agrees with the actual wire definitions", () => expect(() => checkTaggedAnswers(rust, models)).not.toThrow());
  it.each([
    ["kind", models.replace('kind: "date_time"', 'kind: "datetime"')],
    ["missing required field", models.replace('choice_id: string', '')],
    ["optional field", models.replace('choice_id: string', 'choice_id?: string')],
    ["wrong type", models.replace('decimal: string', 'decimal: number')],
    ["extra field", models.replace('decimal: string', 'decimal: string; extra: string')],
    ["unsupported syntax", models.replace('choices: AnswerChoice[]', 'choices: ReadonlyArray<AnswerChoice>')],
  ])("rejects TS %s", (_, mutated) => expect(() => checkTaggedAnswers(rust, mutated)).toThrow());
  it("rejects a changed Rust type", () => expect(() => checkTaggedAnswers(rust.replace('decimal: String', 'decimal: i64'), models)).toThrow());
  it("rejects a changed Rust serde layout", () => expect(() => checkTaggedAnswers(rust.replace('tag = "kind"', 'tag = "type"'), models)).toThrow());
});
