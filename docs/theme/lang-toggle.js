/**
 * lang-toggle.js — the EN/中文 switch shared by BOTH book editions.
 *
 * The deployed book is a two-edition merge (scripts/deploy.cjs --book):
 * the English book at the site root, the zh-CN book under /zh/. This
 * script injects a "switch language" button into mdbook's sticky menu
 * bar (the top-right strip that owns the print/theme/search buttons)
 * on EVERY page — article pages included — navigating to the SAME page
 * in the other edition:
 *
 *   - under /zh/ the prefix is stripped; at the root it is added;
 *   - index.html-aware (/, /index.html and /zh/index.html all map to
 *     the sibling edition's directory index);
 *   - query string and fragment are preserved;
 *   - from file:// (a local `mdbook build` opened straight off disk)
 *     the sibling edition does not exist, so the button renders
 *     disabled with a title saying why.
 *
 * The button always names the language you would GET (中文 while
 * reading English, EN while reading 中文) — a switch, not a status
 * lamp; the current edition is still stated by <html lang> and the
 * title/aria-label. On the zh edition the rendered <html lang="zh_CN">
 * (underscore — the po/ catalog naming that mdbook-gettext derives
 * from book.language) is normalized to the BCP47 "zh-CN" client-side.
 *
 * Plain browser JS, zero dependencies, like theme/rut-book.js. mdbook
 * hashes additional-js filenames, so nothing here may depend on this
 * script's own URL.
 */
(function () {
  "use strict";

  var ZH_SEGMENT = "zh";

  function isZhPath(pathname) {
    var seg = pathname.split("/")[1]; // "" at the root, "zh" under /zh/
    return seg === ZH_SEGMENT;
  }

  /** en paths never carry /index.html; zh index collapses to /zh/ */
  function normalizeEn(pathname) {
    return pathname.replace(/\/index\.html$/, "/") || "/";
  }

  /** the same page in the other edition */
  function siblingUrl(loc) {
    var path = loc.pathname;
    var sibling;
    if (isZhPath(path)) {
      // "/zh/x/y.html" -> "/x/y.html", "/zh/" and "/zh" -> "/"
      sibling = path.slice(("/" + ZH_SEGMENT).length) || "/";
      sibling = normalizeEn(sibling);
    } else {
      var en = normalizeEn(path);
      sibling = "/" + ZH_SEGMENT + (en === "/" ? "/" : en);
    }
    return sibling + loc.search + loc.hash;
  }

  function inject() {
    var loc = window.location;
    var zh = isZhPath(loc.pathname);
    var file = loc.protocol === "file:";
    var bar = document.querySelector(".menu-bar .right-buttons");
    if (!bar) return; // no menu bar (odd embeds) — nothing to attach to

    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "rut-lang-btn";
    btn.textContent = zh ? "EN" : "中文";
    btn.lang = zh ? "en" : "zh-CN";
    btn.title = zh ? "Switch to English" : "切换到中文";
    btn.setAttribute(
      "aria-label",
      zh ? "Switch to English (切换到中文)" : "切换到中文 (Switch to English)"
    );
    if (file) {
      btn.disabled = true;
      btn.title =
        "the other edition only exists in the deployed two-edition merge (scripts/deploy.cjs --book)";
    } else {
      btn.addEventListener("click", function () {
        window.location.href = siblingUrl(loc);
      });
    }
    bar.appendChild(btn);

    if (zh) {
      // zh_CN (catalog naming) -> BCP47 for the DOM
      document.documentElement.setAttribute("lang", "zh-CN");
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", inject);
  } else {
    inject();
  }
})();
