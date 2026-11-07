// Irsty's site: the window tabs, the copy buttons, and the messages that
// arrive in #irsty once when the page loads. Without JavaScript the page
// shows the first window, in full, and the commands to copy by hand.
"use strict";

(() => {
  const tabs = [...document.querySelectorAll('.windows [role="tab"]')];

  function select(tab, focus) {
    for (const other of tabs) {
      const chosen = other === tab;
      other.setAttribute("aria-selected", String(chosen));
      other.tabIndex = chosen ? 0 : -1;
      document.getElementById(other.getAttribute("aria-controls")).hidden = !chosen;
    }
    if (focus) tab.focus();
  }

  for (const [i, tab] of tabs.entries()) {
    tab.addEventListener("click", () => select(tab, false));
    tab.addEventListener("keydown", (event) => {
      const step = { ArrowRight: 1, ArrowLeft: -1 }[event.key];
      if (step) {
        event.preventDefault();
        select(tabs[(i + step + tabs.length) % tabs.length], true);
      } else if (event.key === "Home" || event.key === "End") {
        event.preventDefault();
        select(tabs[event.key === "Home" ? 0 : tabs.length - 1], true);
      }
    });
  }

  // Keys 1 to 6 switch windows, as Alt+1 to Alt+6 would, while the tour is
  // in view and nothing that takes typing has focus.
  const tour = document.getElementById("tour");
  let tourVisible = false;
  if (tour && "IntersectionObserver" in window) {
    new IntersectionObserver(([entry]) => {
      tourVisible = entry.isIntersecting;
    }, { threshold: 0.4 }).observe(tour);
  }
  document.addEventListener("keydown", (event) => {
    if (!tourVisible || event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.target.closest("input, textarea, [contenteditable]")) return;
    const n = Number(event.key);
    if (Number.isInteger(n) && n >= 1 && n <= tabs.length) {
      select(tabs[n - 1], false);
    }
  });

  for (const button of document.querySelectorAll(".copy")) {
    button.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(button.dataset.copy);
        button.textContent = "Copied";
        button.dataset.done = "";
      } catch {
        button.textContent = "Select and copy";
      }
      setTimeout(() => {
        button.textContent = "Copy";
        delete button.dataset.done;
      }, 2000);
    });
  }

  // The arrival: earlier renders of the channel, each with one line fewer,
  // shown in turn before the full one. Once, and not at all for people who
  // ask for less motion.
  const arrive = document.querySelector(".arrive");
  const calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (arrive && !calm) {
    const steps = [...arrive.querySelectorAll(".step")]
      .sort((a, b) => b.dataset.step - a.dataset.step);
    const final = arrive.querySelector(".final");
    const show = (shown) => {
      for (const step of steps) step.hidden = step !== shown;
      final.hidden = shown !== final;
    };
    show(steps[0]);
    let i = 0;
    const next = () => {
      i += 1;
      show(i < steps.length ? steps[i] : final);
      if (i < steps.length) setTimeout(next, 900);
    };
    setTimeout(next, 1200);
  }
})();
