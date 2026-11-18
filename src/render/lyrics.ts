// Layer 4: the lyrics stage.
//
// A small keyed list holds at most three visible items: previous, current and
// next. DOM nodes are reused as items move between roles. Only class names
// change per word, and only when the active word actually changes.

import { activeWordIndex, buildDisplayItems, indexAt, type DisplayItem } from "../sync.ts";
import type { Line } from "../types.ts";

type Role = "prev" | "current" | "next";

type StageNode = {
  item: DisplayItem;
  el: HTMLElement;
  words: HTMLElement[];
  dots: HTMLElement[];
  role: Role | null;
  wordIdx: number;
};

const MOVE_MS = 500;
const EASE = "cubic-bezier(.2,.8,.2,1)";

const WORD_MS = 380;

const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

/**
 * A word that stops being active keeps the serif italic while it shrinks, then
 * swaps to Geist once it is small: the font swap can't be animated, and doing it
 * at 136 px flashes a huge bold word.
 */
function holdSerif(el: HTMLElement): void {
  if (reducedMotion.matches) return;
  el.classList.add("was");
  window.setTimeout(() => el.classList.remove("was"), WORD_MS);
}

export class LyricsStage {
  private readonly stage: HTMLElement;
  private items: DisplayItem[] = [];
  private readonly nodes = new Map<string, StageNode>();
  private cur = -2;

  constructor(stage: HTMLElement) {
    this.stage = stage;
  }

  setLines(lines: Line[], durationMs: number): void {
    this.clear();
    this.items = buildDisplayItems(lines, durationMs);
  }

  clear(): void {
    this.items = [];
    this.nodes.clear();
    this.stage.replaceChildren();
    this.cur = -2;
  }

  /** Called every animation frame with the lyrics-adjusted position. */
  update(t: number): void {
    if (this.items.length === 0) return;
    const c = indexAt(this.items, t);
    if (c !== this.cur) {
      const forward = c > this.cur;
      this.cur = c;
      this.applyRoles(c, forward);
    }
    const item = this.items[c];
    if (!item) return;
    const node = this.nodes.get(item.key);
    if (!node) return;
    if (item.kind === "line") {
      const wi = activeWordIndex(item.words, t);
      if (wi !== node.wordIdx) this.setWordStates(node, wi);
    } else {
      this.updateDots(node.dots, (t - item.startMs) / Math.max(1, item.endMs - item.startMs), t);
    }
  }

  private create(item: DisplayItem): StageNode {
    const el = document.createElement("div");
    el.className = "line";
    const words: HTMLElement[] = [];
    const dots: HTMLElement[] = [];
    if (item.kind === "line") {
      for (const w of item.words) {
        const span = document.createElement("span");
        span.className = "w";
        span.textContent = w.text.trim();
        el.appendChild(span);
        words.push(span);
      }
    } else {
      el.classList.add("gap");
      const wrap = document.createElement("div");
      wrap.className = "dots";
      for (let i = 0; i < 3; i++) {
        const d = document.createElement("i");
        wrap.appendChild(d);
        dots.push(d);
      }
      el.appendChild(wrap);
    }
    return { item, el, words, dots, role: null, wordIdx: -2 };
  }

  private setWordStates(node: StageNode, wi: number): void {
    node.wordIdx = wi;
    const words = node.words;
    for (let i = 0; i < words.length; i++) {
      const cls = i < wi ? "w sung" : i === wi ? "w active" : "w";
      const el = words[i];
      if (el.className === cls) continue;
      const wasActive = el.classList.contains("active");
      el.className = cls;
      if (wasActive) holdSerif(el);
    }
  }

  private clearWordStates(node: StageNode): void {
    node.wordIdx = -2;
    for (const w of node.words) {
      if (w.className === "w") continue;
      const wasActive = w.classList.contains("active");
      w.className = "w";
      if (wasActive) holdSerif(w);
    }
  }

  private applyRoles(c: number, forward: boolean): void {
    const want: [DisplayItem, Role][] = [];
    if (this.items[c - 1]) want.push([this.items[c - 1], "prev"]);
    if (this.items[c]) want.push([this.items[c], "current"]);
    if (this.items[c + 1]) want.push([this.items[c + 1], "next"]);
    const wantKeys = new Set(want.map(([it]) => it.key));
    const animate = !reducedMotion.matches;

    // FLIP: first positions of nodes that stay.
    const first = new Map<string, number>();
    if (animate) {
      for (const [key, n] of this.nodes) {
        if (wantKeys.has(key)) first.set(key, n.el.getBoundingClientRect().top);
      }
    }

    // Nodes that are no longer visible leave (up when moving forward).
    const stageRect = this.stage.getBoundingClientRect();
    for (const [key, n] of [...this.nodes]) {
      if (wantKeys.has(key)) continue;
      this.nodes.delete(key);
      this.leave(n, stageRect, forward ? "up" : "down", animate);
    }

    // Create / re-role in order: prev, current, next.
    let prevEl: HTMLElement | null = null;
    const entered: HTMLElement[] = [];
    for (const [item, role] of want) {
      let n = this.nodes.get(item.key);
      if (!n) {
        n = this.create(item);
        this.nodes.set(item.key, n);
        if (animate) {
          n.el.classList.add("enter");
          entered.push(n.el);
        }
      }
      if (n.role !== role) {
        n.el.classList.remove("prev", "current", "next");
        n.el.classList.add(role);
        n.role = role;
        if (role !== "current") this.clearWordStates(n);
        else n.wordIdx = -2;
      }
      // Keep DOM order without touching nodes that are already in place.
      const expectedPrev: Element | null = prevEl;
      if (!n.el.isConnected || this.prevLiveSibling(n.el) !== expectedPrev) {
        const ref: ChildNode | null = expectedPrev ? expectedPrev.nextSibling : this.stage.firstChild;
        this.stage.insertBefore(n.el, ref);
      }
      prevEl = n.el;
    }

    if (!animate) return;

    // FLIP: invert + play with WAAPI so it never fights the CSS transitions.
    for (const [key, top] of first) {
      const n = this.nodes.get(key);
      if (!n) continue;
      const dy = top - n.el.getBoundingClientRect().top;
      if (Math.abs(dy) < 0.5) continue;
      n.el.animate([{ transform: `translateY(${dy}px)` }, { transform: "translateY(0)" }], {
        duration: MOVE_MS,
        easing: EASE,
      });
    }
    if (entered.length) {
      void this.stage.offsetHeight; // commit the .enter state before releasing it
      for (const el of entered) el.classList.remove("enter");
    }
  }

  private prevLiveSibling(el: Element): Element | null {
    let p = el.previousElementSibling;
    while (p && p.classList.contains("leaving")) p = p.previousElementSibling;
    return p;
  }

  private leave(n: StageNode, stageRect: DOMRect, dir: "up" | "down", animate: boolean): void {
    const el = n.el;
    if (!animate || !el.isConnected) {
      el.remove();
      return;
    }
    const r = el.getBoundingClientRect();
    el.style.top = `${r.top - stageRect.top}px`;
    el.style.left = `${r.left - stageRect.left}px`;
    el.style.width = `${r.width}px`;
    el.classList.add("leaving");
    void el.offsetHeight;
    el.classList.add(dir === "up" ? "leave-up" : "leave-down");
    window.setTimeout(() => el.remove(), MOVE_MS + 40);
  }

  private updateDots(dots: HTMLElement[], progress: number, t: number): void {
    const p = Math.min(1, Math.max(0, progress));
    const still = reducedMotion.matches;
    for (let i = 0; i < dots.length; i++) {
      const fill = Math.min(1, Math.max(0, p * 3 - i));
      const breathe = still ? 0 : Math.sin(t / 420 + i * 0.9) * 0.06;
      dots[i].style.opacity = (0.25 + 0.75 * fill).toFixed(3);
      dots[i].style.transform = still ? "" : `scale(${(0.82 + 0.26 * fill + breathe).toFixed(3)})`;
    }
  }
}

/** Instrumental card dots: same look, driven by song progress. */
export function updateCardDots(dots: HTMLElement[], progress: number, t: number): void {
  const still = reducedMotion.matches;
  const p = Math.min(1, Math.max(0, progress));
  for (let i = 0; i < dots.length; i++) {
    const phase = Math.sin(t / 600 - i * 0.8) * 0.5 + 0.5;
    dots[i].style.opacity = (0.3 + 0.5 * phase + 0.2 * p).toFixed(3);
    if (!still) dots[i].style.transform = `scale(${(0.8 + 0.25 * phase).toFixed(3)})`;
  }
}

// ------------------------------------------------------------------ plain lyrics

export class PlainLyrics {
  private readonly viewport: HTMLElement;
  private readonly inner: HTMLElement;
  private lastY = -1;
  private range = 0;

  constructor(viewport: HTMLElement, inner: HTMLElement) {
    this.viewport = viewport;
    this.inner = inner;
  }

  set(text: string): void {
    this.inner.replaceChildren();
    for (const raw of text.split(/\r?\n/)) {
      const p = document.createElement("p");
      const line = raw.trim();
      if (line) p.textContent = line;
      else p.className = "blank";
      this.inner.appendChild(p);
    }
    this.lastY = -1;
    this.measure();
  }

  measure(): void {
    this.range = Math.max(0, this.inner.scrollHeight - this.viewport.clientHeight);
  }

  /** Slow auto-scroll paced to the song: start → first line centred, end → last line centred. */
  update(t: number, durationMs: number): void {
    if (!this.range) this.measure();
    // Hold the first lines for a moment and arrive a little before the end.
    const lead = Math.min(15000, durationMs * 0.08);
    const p = durationMs > 0 ? Math.min(1, Math.max(0, (t - lead) / Math.max(1, durationMs - lead * 2))) : 0;
    const y = Math.round(p * this.range * 2) / 2;
    if (y === this.lastY) return;
    this.lastY = y;
    this.inner.style.transform = `translate(-50%, ${-y}px)`;
  }
}
