// Tiny client-side router for the LUAR docs.
// Each topic lives in pages/<id>.html as a plain HTML fragment. We fetch them
// all once, cache them, index every element id, and swap fragments into #content
// when the URL hash changes. Sub-section anchors resolve to the page that holds
// them and scroll into view.

var PAGES = [
  "overview", "getting-started", "syntax", "variables", "scope", "buff", "functions",
  "control-flow", "tables", "classes", "enums", "modules",
  "stdlib", "builtins", "ferrite", "gc", "precompilation",
  "host-api", "type-rules"
];

var cache = {};        // page id -> html string
var anchorPage = {};   // any element id -> the page that contains it
var ready = false;

function highlight(page) {
  var links = document.querySelectorAll("nav.sidebar a");
  for (var i = 0; i < links.length; i++) {
    var a = links[i];
    if (a.getAttribute("href") === "#" + page) {
      a.classList.add("active");
    } else {
      a.classList.remove("active");
    }
  }
}

function navigate() {
  if (!ready) return;
  var hash = (location.hash || "#overview").slice(1);
  var page = PAGES.indexOf(hash) !== -1 ? hash : (anchorPage[hash] || "overview");
  document.getElementById("content").innerHTML =
    cache[page] || "<h1>Not found</h1><p>That page does not exist.</p>";
  highlight(page);
  if (PAGES.indexOf(hash) === -1) {
    var el = document.getElementById(hash);
    if (el) {
      el.scrollIntoView();
      return;
    }
  }
  window.scrollTo(0, 0);
}

function init() {
  var pending = PAGES.length;
  var failed = false;
  PAGES.forEach(function (id) {
    fetch("pages/" + id + ".html")
      .then(function (res) { return res.ok ? res.text() : ""; })
      .catch(function () { failed = true; return ""; })
      .then(function (text) {
        cache[id] = text;
        if (text) {
          var tmp = document.createElement("div");
          tmp.innerHTML = text;
          var withId = tmp.querySelectorAll("[id]");
          for (var i = 0; i < withId.length; i++) {
            anchorPage[withId[i].id] = id;
          }
        }
        pending -= 1;
        if (pending === 0) {
          ready = true;
          if (failed && !cache["overview"]) {
            document.getElementById("content").innerHTML =
              "<h1>Cannot load pages</h1><p>The documentation loads its pages over HTTP. " +
              "On GitHub Pages this just works. To preview locally, serve the folder " +
              "with a static web server instead of opening the file directly.</p>";
            return;
          }
          navigate();
        }
      });
  });
}

window.addEventListener("hashchange", navigate);
init();
