/**
 * lang-toggle.js — the EN | 中文 switch shared by BOTH book editions.
 *
 * The deployed book is a two-edition merge (scripts/deploy.cjs --book):
 * the English book at the site root, the zh-CN book under /zh/. This
 * script injects a small fixed control into every page linking to the
 * SAME page in the other edition:
 *
 *   - under /zh/ the prefix is stripped; at the root it is added;
 *   - index.html-aware (/, /index.html and /zh/index.html all map to
 *     the sibling edition's directory index);
 *   - query string and fragment are preserved;
 *   - from file:// (a local `mdbook build` opened straight off disk)
 *     the sibling edition does not exist, so the control degrades to a
 *     no-op: both halves render inert, with no link to a dead path.
 *
 * The current edition is aria-current, not a self link. On the zh
 * edition the rendered <html lang="zh_CN"> (underscore — the po/
 * catalog naming that mdbook-gettext derives from book.language) is
 * normalized to the BCP47 "zh-CN" client-side.
 *
 * Plain browser JS, zero dependencies, like theme/rut-book.js. mdbook
 * hashes additional-js filenames, so nothing here may depend on this
 * script's own URL.
 *
 * Placement: fixed bottom-right (see the .rut-lang block in
 * theme/rut-book.css) — mdbook's sticky menu bar owns the top strip
 * (search/theme buttons) and full-height prev/next arrow strips own
 * the side edges, so the corner is the one spot that never collides.
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

  function el(tag, attrs, text) {
    var node = document.createElement(tag);
    for (var k in attrs) node.setAttribute(k, attrs[k]);
    if (text != null) node.textContent = text;
    return node;
  }

  function inject() {
    var loc = window.location;
    var zh = isZhPath(loc.pathname);
    var file = loc.protocol === "file:";
    var href = siblingUrl(loc);

    var nav = el("nav", { "class": "rut-lang", "aria-label": zh ? "语言" : "Language" });

    var make = function (label, lang, current) {
      if (current || file) {
        var span = el("span", { "class": "rut-lang-item" + (current ? " rut-lang-current" : ""), lang: lang }, label);
        if (current) span.setAttribute("aria-current", "true");
        return span;
      }
      return el("a", {
        "class": "rut-lang-item",
        href: href,
        lang: lang,
        hreflang: lang,
        title: zh ? "Switch to English" : "切换到中文",
      }, label);
    };

    nav.appendChild(make("EN", "en", !zh));
    nav.appendChild(el("span", { "class": "rut-lang-sep", "aria-hidden": "true" }, "|"));
    nav.appendChild(make("中文", "zh-CN", zh));

    document.body.appendChild(nav);

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
