/**
 * Checking and formatting a JSON body as text. Bodies are templates, so `{ "id": {{seq}} }` is a
 * fine body even though it isn't JSON until the engine fills it in: an unquoted `{{…}}` counts as a
 * value here. Works on tokens rather than `JSON.parse`, so formatting keeps every value exactly as
 * written (`1.0`, big integers, escapes) and only changes whitespace.
 */

type Punct = "{" | "}" | "[" | "]" | ":" | ",";

type Token =
  | { kind: "punct"; text: Punct; start: number }
  | { kind: "string"; text: string; start: number }
  /** A number, `true`/`false`/`null`, or anything holding a `{{…}}` template. */
  | { kind: "bare"; text: string; start: number; template: boolean };

export interface JsonProblem {
  message: string;
  /** 1-based, for showing; `offset` is for putting the caret there. */
  line: number;
  column: number;
  offset: number;
}

const PUNCT = new Set<string>(["{", "}", "[", "]", ":", ","]);
const NUMBER = /^-?(0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?$/;
const LITERALS = new Set(["true", "false", "null"]);

class Problem extends Error {
  constructor(
    message: string,
    readonly offset: number,
  ) {
    super(message);
  }
}

const describe = (t: Token | undefined) =>
  t === undefined ? "the end" : t.kind === "punct" ? `"${t.text}"` : `\`${t.text}\``;

function tokenize(text: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (/\s/.test(ch)) {
      i++;
    } else if (ch === '"') {
      const start = i++;
      while (i < text.length && text[i] !== '"') {
        if (text[i] === "\n") throw new Problem("This string isn't closed on its line", start);
        i += text[i] === "\\" ? 2 : 1;
      }
      if (i >= text.length) throw new Problem("This string isn't closed", start);
      tokens.push({ kind: "string", text: text.slice(start, ++i), start });
    } else if (PUNCT.has(ch) && !text.startsWith("{{", i)) {
      tokens.push({ kind: "punct", text: ch as Punct, start: i++ });
    } else {
      // A bare value, which may be or contain templates: `42`, `{{seq}}`, `{{base}}0`.
      const start = i;
      let template = false;
      while (i < text.length) {
        if (text.startsWith("{{", i)) {
          const end = text.indexOf("}}", i + 2);
          if (end < 0) throw new Problem("This {{ template isn't closed with }}", i);
          template = true;
          i = end + 2;
        } else if (!/\s/.test(text[i]) && !PUNCT.has(text[i]) && text[i] !== '"') {
          i++;
        } else {
          break;
        }
      }
      const value = text.slice(start, i);
      if (!template && !NUMBER.test(value) && !LITERALS.has(value)) {
        const hint = /^'/.test(value) ? " (JSON strings use double quotes)" : "";
        throw new Problem(`Unexpected \`${value}\`${hint}`, start);
      }
      tokens.push({ kind: "bare", text: value, start, template });
    }
  }
  return tokens;
}

/** Checks the structure: one value, objects with string keys, no trailing commas. */
function parse(tokens: Token[], end: number) {
  let pos = 0;
  const at = () => tokens[pos];
  const offsetOf = (t: Token | undefined) => t?.start ?? end;
  const is = (t: Token | undefined, p: Punct) => t?.kind === "punct" && t.text === p;

  const value = (): void => {
    const t = at();
    if (t === undefined) throw new Problem("Expected a value", end);
    if (t.kind !== "punct") {
      pos++;
      return;
    }
    if (t.text === "{") return container("}", true);
    if (t.text === "[") return container("]", false);
    throw new Problem(`Expected a value, found ${describe(t)}`, t.start);
  };

  const container = (close: "}" | "]", object: boolean) => {
    pos++;
    if (is(at(), close)) {
      pos++;
      return;
    }
    for (;;) {
      if (object) {
        const key = at();
        // An unquoted template key is allowed: it's whatever the template renders.
        const keyOk = key?.kind === "string" || (key?.kind === "bare" && key.template);
        if (!keyOk) {
          // `pos - 1` is the comma, unless this is the first key of an object.
          if (is(key, "}")) throw new Problem("Remove the trailing comma", tokens[pos - 1].start);
          throw new Problem(`Expected a "quoted" key, found ${describe(key)}`, offsetOf(key));
        }
        pos++;
        if (!is(at(), ":")) throw new Problem(`Expected ":" after the key`, offsetOf(at()));
        pos++;
      } else if (is(at(), "]")) {
        throw new Problem("Remove the trailing comma", tokens[pos - 1].start);
      }
      value();
      const next = at();
      if (is(next, ",")) {
        pos++;
        continue;
      }
      if (is(next, close)) {
        pos++;
        return;
      }
      throw new Problem(`Expected "," or "${close}", found ${describe(next)}`, offsetOf(next));
    }
  };

  value();
  if (pos < tokens.length) {
    throw new Problem(`Unexpected ${describe(at())} after the end of the JSON`, offsetOf(at()));
  }
}

const position = (text: string, offset: number) => {
  const before = text.slice(0, offset);
  return { line: before.split("\n").length, column: offset - before.lastIndexOf("\n") };
};

/** The first problem in a JSON body, or `null` if it's fine (or empty: no body is fine too). */
export function checkJson(text: string): JsonProblem | null {
  if (text.trim() === "") return null;
  try {
    parse(tokenize(text), text.length);
    return null;
  } catch (err) {
    if (!(err instanceof Problem)) throw err;
    return { message: err.message, ...position(text, err.offset), offset: err.offset };
  }
}

/** Re-indents a valid JSON body with `indent`, keeping every value as written. `null` if invalid. */
export function formatJson(text: string, indent = "  "): string | null {
  if (checkJson(text) !== null || text.trim() === "") return null;
  const tokens = tokenize(text);
  let out = "";
  let depth = 0;
  const newline = () => "\n" + indent.repeat(depth);
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (t.kind !== "punct") {
      out += t.text;
      continue;
    }
    switch (t.text) {
      case "{":
      case "[": {
        const close = t.text === "{" ? "}" : "]";
        const next = tokens[i + 1];
        if (next?.kind === "punct" && next.text === close) {
          out += t.text + close;
          i++;
        } else {
          depth++;
          out += t.text + newline();
        }
        break;
      }
      case "}":
      case "]":
        depth--;
        out += newline() + t.text;
        break;
      case ",":
        out += "," + newline();
        break;
      case ":":
        out += ": ";
        break;
    }
  }
  return out;
}
