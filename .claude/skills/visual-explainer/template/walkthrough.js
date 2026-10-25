// visual-explainer walkthrough: focus one report section at a time and show its commentary.
// The steps come from <script id="ve-walkthrough">: [{ scene, target, text }], where target is the
// data-shot id of the section, or null for a step with no section (a centered card).
// Start: the "Start the walkthrough" button, or the URL fragment #walkthrough (step 1) or
// #walkthrough-<n> (step n).
// Keys: right arrow or Enter = next, left arrow = back, Escape = close.
(function () {
  "use strict";
  var data = document.getElementById("ve-walkthrough");
  var startBtn = document.querySelector("[data-walkthrough-start]");
  if (!data || !startBtn) return;
  var steps = JSON.parse(data.textContent);
  var TITLES = { intro: "Overview", tldr: "TL;DR", outro: "Wrap-up" };
  var GAP = 12;
  var PAD = 8;
  var shade = null, spot = null, card = null, index = 0, lastFocus = null;

  startBtn.addEventListener("click", function () { open(0); });
  var hash = /^#walkthrough(?:-(\d+))?$/.exec(location.hash);
  if (hash) open(hash[1] ? Number(hash[1]) - 1 : 0);

  function open(i) {
    if (document.documentElement.classList.contains("shot")) return;
    lastFocus = document.activeElement;
    shade = el("div", "wt-shade");
    spot = el("div", "wt-spot");
    card = el("div", "wt-card");
    card.setAttribute("role", "dialog");
    card.setAttribute("aria-modal", "true");
    card.setAttribute("aria-labelledby", "wt-title");
    shade.addEventListener("click", close);
    shade.appendChild(spot);
    document.body.appendChild(shade);
    document.body.appendChild(card);
    document.addEventListener("keydown", onKey, true);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, { passive: true });
    show(i);
  }

  function close() {
    if (!card) return;
    document.removeEventListener("keydown", onKey, true);
    window.removeEventListener("resize", place);
    window.removeEventListener("scroll", place);
    shade.remove();
    card.remove();
    shade = spot = card = null;
    if (lastFocus && lastFocus.focus) lastFocus.focus();
  }

  function show(i) {
    index = Math.max(0, Math.min(steps.length - 1, i));
    var step = steps[index];
    var target = targetOf(step);
    card.innerHTML = "";
    var x = el("button", "wt-x");
    x.type = "button";
    x.setAttribute("aria-label", "Close the walkthrough");
    x.textContent = "×";
    x.addEventListener("click", close);
    var count = el("p", "wt-step");
    count.textContent = "Step " + (index + 1) + " of " + steps.length;
    var title = el("h2");
    title.id = "wt-title";
    title.textContent = titleOf(step, target);
    var text = el("p");
    text.textContent = step.text;
    var nav = el("div", "wt-nav");
    var dots = el("span", "wt-dots");
    dots.setAttribute("aria-hidden", "true");
    for (var d = 0; d < steps.length; d += 1) dots.appendChild(el("i", d === index ? "on" : ""));
    var back = button("Back", "wt-back", function () { show(index - 1); });
    back.disabled = index === 0;
    var last = index === steps.length - 1;
    var next = button(last ? "Finish" : "Next", "wt-next", last ? close : function () { show(index + 1); });
    nav.appendChild(dots);
    nav.appendChild(back);
    nav.appendChild(next);
    [x, count, title, text, nav].forEach(function (n) { card.appendChild(n); });
    if (target) {
      var details = target.closest("details");
      if (details && !details.open) details.open = true;
      var reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      target.scrollIntoView({ block: target.offsetHeight + 2 * PAD > window.innerHeight * 0.6 ? "start" : "center", behavior: reduce ? "auto" : "smooth" });
    }
    place();
    next.focus();
  }

  // Put the spotlight on the target and the card below it, else above it, else at the bottom
  // right of the viewport. A step with no target gets a centered card and a full shade.
  function place() {
    if (!card) return;
    var target = targetOf(steps[index]);
    var vw = window.innerWidth, vh = window.innerHeight;
    var cw = card.offsetWidth, ch = card.offsetHeight;
    shade.classList.toggle("has-spot", Boolean(target));
    spot.hidden = !target;
    if (!target) {
      setPos(card, (vw - cw) / 2, (vh - ch) / 2);
      return;
    }
    var r = target.getBoundingClientRect();
    var top = r.top - PAD, left = r.left - PAD, w = r.width + 2 * PAD, h = r.height + 2 * PAD;
    spot.style.top = top + "px";
    spot.style.left = left + "px";
    spot.style.width = w + "px";
    spot.style.height = h + "px";
    var x = Math.max(16, Math.min(r.left, vw - cw - 16));
    var below = top + h + GAP, above = top - GAP - ch;
    if (below >= 16 && below + ch <= vh - 16) setPos(card, x, below);
    else if (above >= 16 && above + ch <= vh - 16) setPos(card, x, above);
    else setPos(card, vw - cw - 16, Math.max(16, vh - ch - 16));
  }

  function onKey(e) {
    if (e.key === "Escape") close();
    else if (e.key === "ArrowRight" || (e.key === "Enter" && e.target === document.body)) show(index + 1);
    else if (e.key === "ArrowLeft") show(index - 1);
    else if (e.key === "Tab") trapTab(e);
    else return;
    if (e.key !== "Tab") e.preventDefault();
  }

  // Keep the keyboard focus in the card while it is open.
  function trapTab(e) {
    var items = card.querySelectorAll("button:not([disabled])");
    if (!items.length) return;
    var first = items[0], last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) { last.focus(); e.preventDefault(); }
    else if (!e.shiftKey && document.activeElement === last) { first.focus(); e.preventDefault(); }
  }

  function targetOf(step) {
    return step.target ? document.querySelector('[data-shot="' + step.target + '"]') : null;
  }

  // The heading of the section without its number, else a fixed title for the scene.
  function titleOf(step, target) {
    if (TITLES[step.scene]) return TITLES[step.scene];
    var h = target && target.querySelector("h2");
    if (!h) return step.scene;
    var n = h.querySelector(".n");
    return h.textContent.slice(n ? n.textContent.length : 0).trim();
  }

  function setPos(node, x, y) {
    node.style.left = Math.round(x) + "px";
    node.style.top = Math.round(y) + "px";
  }

  function button(label, cls, onClick) {
    var b = el("button", cls);
    b.type = "button";
    b.textContent = label;
    b.addEventListener("click", onClick);
    return b;
  }

  function el(tag, cls) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    return node;
  }
})();
