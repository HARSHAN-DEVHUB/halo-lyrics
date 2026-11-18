// Idle screen: big clock + date over the dimmed artwork.

export class IdleScreen {
  private readonly clock: HTMLElement;
  private readonly date: HTMLElement;
  private lastText = "";

  constructor(clock: HTMLElement, date: HTMLElement) {
    this.clock = clock;
    this.date = date;
  }

  update(now = new Date()): void {
    const time = now.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
    if (time === this.lastText) return;
    this.lastText = time;
    this.clock.textContent = time;
    this.date.textContent = now.toLocaleDateString(undefined, {
      weekday: "long",
      day: "numeric",
      month: "long",
    });
  }
}
