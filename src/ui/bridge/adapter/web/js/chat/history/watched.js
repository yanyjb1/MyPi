// The gaps a page leaves for rounds it has not read: each asks for its own ranges the moment it is near.

import { at } from "../../base/dom.js";

export const watched = {
  seen: new Map(),
  observer: null,
  observe(node, act) {
    if (!this.observer) {
      this.observer = new IntersectionObserver((rows) => {
        for (const row of rows) {
          if (!row.isIntersecting) continue;
          const run = this.seen.get(row.target);
          if (run) {
            this.seen.delete(row.target);
            this.observer.unobserve(row.target);
            run();
          }
        }
      }, { root: at("chat"), rootMargin: "120px" });
    }
    this.seen.set(node, act);
    this.observer.observe(node);
  },
};
