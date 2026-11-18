// Layer 1: the drifting, blurred artwork. Crossfades over 1.2 s on change.

const FADE_MS = 1200;

export class Background {
  private readonly root: HTMLElement;
  private current: { el: HTMLElement; url: string } | null = null;

  constructor(root: HTMLElement) {
    this.root = root;
  }

  set(url: string | undefined): void {
    if (!url) return; // keep the last artwork (the idle screen wants it too)
    if (this.current?.url === url) return;

    const layer = document.createElement("div");
    layer.className = "bg-layer";
    const drift = document.createElement("div");
    drift.className = "bg-drift";
    // Continue the drift from where the old layer was so the swap doesn't jump.
    const prevAnim = this.current?.el.querySelector<HTMLElement>(".bg-drift")?.getAnimations()[0];
    const img = new Image();
    img.decoding = "async";
    img.alt = "";
    img.src = url;
    drift.appendChild(img);
    layer.appendChild(drift);

    this.current = { el: layer, url };

    const show = () => {
      // A newer artwork arrived while this one was decoding: drop this one,
      // otherwise it would be inserted on top and stay there.
      if (this.current?.el !== layer) return;
      // The dim overlay is the first child; artwork layers go before it.
      this.root.insertBefore(layer, this.root.querySelector(".bg-dim"));
      const anim = drift.getAnimations()[0];
      if (anim && prevAnim?.currentTime != null) anim.currentTime = prevAnim.currentTime;
      requestAnimationFrame(() => {
        layer.classList.add("on");
        // Fade out every other layer (not just the previous one), so layers
        // skipped by a fast track change can't pile up.
        for (const el of this.root.querySelectorAll<HTMLElement>(".bg-layer")) {
          if (el === layer || el.dataset.leaving) continue;
          el.dataset.leaving = "1";
          el.classList.remove("on");
          window.setTimeout(() => el.remove(), FADE_MS + 100);
        }
      });
    };
    if (img.complete) show();
    else img.decode().then(show, show);
  }
}
