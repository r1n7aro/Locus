import { describe, expect, it } from "vitest";
import { StreamingTextChunks } from "../composables/streamingTextChunks";

function fill(deltas: string[]): StreamingTextChunks {
  const chunks = new StreamingTextChunks();
  for (const delta of deltas) chunks.append(delta);
  return chunks;
}

describe("StreamingTextChunks", () => {
  it("accumulates deltas and reports length", () => {
    const chunks = fill(["hello", " ", "world"]);
    expect(chunks.length).toBe(11);
    expect(chunks.full()).toBe("hello world");
  });

  it("ignores empty appends without bumping the version", () => {
    const chunks = new StreamingTextChunks();
    const before = chunks.version.value;
    chunks.append("");
    expect(chunks.version.value).toBe(before);
    expect(chunks.length).toBe(0);
  });

  it("bumps the version on every non-empty append", () => {
    const chunks = new StreamingTextChunks();
    const before = chunks.version.value;
    chunks.append("a");
    chunks.append("b");
    expect(chunks.version.value).toBe(before + 2);
  });

  it("tracks non-whitespace presence", () => {
    const chunks = new StreamingTextChunks();
    expect(chunks.hasNonWhitespace).toBe(false);
    chunks.append("  \n\t");
    expect(chunks.hasNonWhitespace).toBe(false);
    chunks.append("  x");
    expect(chunks.hasNonWhitespace).toBe(true);
  });

  it("freezes parts as they reach the target size and keeps an active tail", () => {
    const chunks = new StreamingTextChunks();
    chunks.append("a".repeat(5000));
    expect(chunks.frozenParts.length).toBe(1);
    expect(chunks.activePart).toBe("");
    chunks.append("tail");
    expect(chunks.frozenParts.length).toBe(1);
    expect(chunks.activePart).toBe("tail");
    expect(chunks.full()).toBe("a".repeat(5000) + "tail");
  });

  it("keeps frozen part indexes stable across growth", () => {
    const chunks = new StreamingTextChunks();
    chunks.append("x".repeat(4096));
    const first = chunks.frozenParts[0];
    chunks.append("y".repeat(4096));
    expect(chunks.frozenParts[0]).toBe(first);
    expect(chunks.frozenParts.length).toBe(2);
  });

  it("caches full() until the next append", () => {
    const chunks = fill(["one", "two"]);
    const first = chunks.full();
    expect(chunks.full()).toBe(first);
    chunks.append("three");
    expect(chunks.full()).toBe("onetwothree");
  });

  it("reads from an offset across frozen and active parts", () => {
    const chunks = new StreamingTextChunks();
    const text = "abcdefgh".repeat(1024) + "live";
    for (let i = 0; i < text.length; i += 100) {
      chunks.append(text.slice(i, i + 100));
    }
    expect(chunks.readFrom(0)).toBe(text);
    expect(chunks.readFrom(5)).toBe(text.slice(5));
    expect(chunks.readFrom(4200)).toBe(text.slice(4200));
    expect(chunks.readFrom(text.length - 2)).toBe(text.slice(-2));
    expect(chunks.readFrom(text.length)).toBe("");
    expect(chunks.readFrom(text.length + 5)).toBe("");
  });

  it("reads arbitrary ranges identically to String.slice", () => {
    const text = Array.from({ length: 300 }, (_, i) => `line ${i} of the stream\n`).join("");
    const chunks = new StreamingTextChunks();
    for (let i = 0; i < text.length; i += 37) {
      chunks.append(text.slice(i, i + 37));
    }
    const probes: Array<[number, number]> = [
      [0, 0],
      [0, 1],
      [0, text.length],
      [10, 20],
      [4090, 4100],
      [4096, 8192],
      [8000, 8003],
      [text.length - 5, text.length],
      [50, 5000],
    ];
    for (const [start, end] of probes) {
      expect(chunks.readRange(start, end)).toBe(text.slice(start, end));
    }
    // Out-of-bounds clamps.
    expect(chunks.readRange(-5, 10)).toBe(text.slice(0, 10));
    expect(chunks.readRange(text.length - 3, text.length + 10)).toBe(text.slice(-3));
    expect(chunks.readRange(20, 10)).toBe("");
  });

  it("reset clears content, bumps generation, and bumps version", () => {
    const chunks = fill(["some", "text"]);
    const generation = chunks.generation;
    const version = chunks.version.value;
    chunks.reset();
    expect(chunks.length).toBe(0);
    expect(chunks.full()).toBe("");
    expect(chunks.frozenParts.length).toBe(0);
    expect(chunks.activePart).toBe("");
    expect(chunks.hasNonWhitespace).toBe(false);
    expect(chunks.generation).toBe(generation + 1);
    expect(chunks.version.value).toBe(version + 1);
  });

  it("matches the plain accumulated string for randomized delta sequences", () => {
    // Deterministic pseudo-random source keeps the test reproducible.
    let seed = 42;
    const nextInt = (bound: number) => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed % bound;
    };
    const alphabet = "abc \n中文字符🚀defgh";
    const chunks = new StreamingTextChunks();
    let expected = "";
    for (let round = 0; round < 500; round += 1) {
      const size = nextInt(200) + 1;
      let delta = "";
      for (let i = 0; i < size; i += 1) {
        delta += alphabet[nextInt(alphabet.length)]!;
      }
      chunks.append(delta);
      expected += delta;
      if (round % 97 === 0) {
        const start = nextInt(expected.length);
        const end = start + nextInt(expected.length - start + 1);
        expect(chunks.readRange(start, end)).toBe(expected.slice(start, end));
      }
    }
    expect(chunks.length).toBe(expected.length);
    expect(chunks.full()).toBe(expected);
    expect(chunks.readFrom(1234)).toBe(expected.slice(1234));
  });
});
