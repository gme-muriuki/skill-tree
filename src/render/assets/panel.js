/* skill-tree embed: toolbar, legend filters, side-panel, pan/zoom.
   Inlined into the generated HTML by src/render/html.rs.

   Each clicked SVG node is joined to its issue record via the node's
   <title> (= NodeId), looked up in the embedded JSON map. Clicking opens
   the panel rather than navigating; the GitHub link moves into the panel.
   body_html is rendered + sanitized at generation time, so it is injected
   as-is. Pan/zoom is a CSS transform on the SVG (no external dependency).
   Search-# and the status filter dim non-matching nodes (layout stays). */

(function () {
  function esc(s) {
    return String(s).replace(/[&<>"]/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c];
    });
  }

  // --- fit-to-view + pan/zoom over the inlined SVG -----------------------
  function setupZoom(stage, svg) {
    svg.removeAttribute("width");
    svg.removeAttribute("height");
    svg.setAttribute("preserveAspectRatio", "xMidYMid meet"); // whole tree fits
    svg.style.width = "100%";
    svg.style.height = "100%";

    var k = 1, tx = 0, ty = 0;
    var MIN = 0.3, MAX = 8;

    function apply() {
      svg.style.transform = "translate(" + tx + "px," + ty + "px) scale(" + k + ")";
    }
    function fit() { k = 1; tx = 0; ty = 0; apply(); }
    function zoomAt(px, py, factor) {
      var nk = Math.min(MAX, Math.max(MIN, k * factor));
      if (nk === k) return;
      tx = px - (px - tx) * (nk / k);
      ty = py - (py - ty) * (nk / k);
      k = nk;
      apply();
    }
    function zoomCenter(factor) {
      var r = stage.getBoundingClientRect();
      zoomAt(r.width / 2, r.height / 2, factor);
    }

    stage.addEventListener("wheel", function (e) {
      e.preventDefault();
      var r = stage.getBoundingClientRect();
      zoomAt(e.clientX - r.left, e.clientY - r.top, Math.exp(-e.deltaY * 0.0015));
    }, { passive: false });

    var dragging = false, moved = false, sx = 0, sy = 0;
    stage.addEventListener("mousedown", function (e) {
      if (e.button !== 0) return;
      dragging = true; moved = false; sx = e.clientX; sy = e.clientY;
      stage.classList.add("st-dragging");
    });
    window.addEventListener("mousemove", function (e) {
      if (!dragging) return;
      var dx = e.clientX - sx, dy = e.clientY - sy;
      if (!moved && (Math.abs(dx) > 4 || Math.abs(dy) > 4)) moved = true;
      if (moved) { tx += dx; ty += dy; sx = e.clientX; sy = e.clientY; apply(); }
    });
    window.addEventListener("mouseup", function () {
      if (!dragging) return;
      dragging = false;
      stage.classList.remove("st-dragging");
      // let click handlers (fired right after) know this was a drag, not a click
      stage._stMoved = moved;
      setTimeout(function () { stage._stMoved = false; }, 0);
    });

    return { fit: fit, zoomIn: function () { zoomCenter(1.3); }, zoomOut: function () { zoomCenter(1 / 1.3); } };
  }

  function buildToolbar(stage, zoom) {
    var tb = document.createElement("div");
    tb.className = "st-toolbar";
    function btn(label, title, fn) {
      var b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.title = title;
      b.addEventListener("click", function (e) { e.stopPropagation(); fn(); });
      tb.appendChild(b);
    }
    btn("+", "Zoom in", zoom.zoomIn);
    btn("−", "Zoom out", zoom.zoomOut);
    btn("Fit", "Fit whole tree", zoom.fit);
    stage.appendChild(tb);
  }

  function initWidget(widget) {
    if (widget.dataset.stInit) return; // guard against double init
    widget.dataset.stInit = "1";

    var stage = widget.querySelector(".st-stage");
    var svg = widget.querySelector("svg");
    var panel = widget.querySelector(".st-panel");
    var dataEl = widget.querySelector("script.st-data");
    if (!stage || !svg || !panel) return;

    var data = {};
    if (dataEl) {
      try { data = JSON.parse(dataEl.textContent || "{}"); } catch (e) { data = {}; }
    }
    var emptyHTML = panel.innerHTML;
    var selected = null;

    var zoom = setupZoom(stage, svg);
    buildToolbar(stage, zoom);

    function clear() {
      if (selected) selected.classList.remove("st-selected");
      selected = null;
      panel.innerHTML = emptyHTML;
    }

    // relationship list; each neighbor resolved via the map. `marked`
    // shows a done/pending check (DEPENDS ON / BLOCKS); without it the
    // list is neutral (RELATED — decorative cross-refs/see-also).
    function relSection(label, ids, marked) {
      if (!ids || !ids.length) return "";
      var items = ids.map(function (rid) {
        var r = data[rid];
        var title = r ? r.title : rid;
        var mark = "";
        if (marked) {
          var done = r && r.state && r.state !== "OPEN";
          mark = done
            ? '<span class="st-rel-mark st-rel-done">✓</span>'
            : '<span class="st-rel-mark st-rel-open">•</span>';
        }
        return '<li class="st-rel-item">' + mark + "<span>" + esc(title) + "</span></li>";
      }).join("");
      return '<div class="st-rel"><div class="st-rel-label">' + label +
             '</div><ul class="st-rel-list">' + items + "</ul></div>";
    }

    function show(g, id) {
      var rec = data[id];
      if (!rec) return;

      if (selected) selected.classList.remove("st-selected");
      selected = g;
      g.classList.add("st-selected");

      var html = '<h2 class="st-title">' + esc(rec.title) + "</h2>";

      html += '<div class="st-badges">';
      if (rec.state === "OPEN") html += '<span class="st-badge st-badge-open">OPEN</span>';
      else if (rec.state === "CLOSED") html += '<span class="st-badge st-badge-closed">CLOSED</span>';
      else if (rec.state) html += '<span class="st-badge st-badge-status">' + esc(rec.state) + "</span>";
      if (rec.status) html += '<span class="st-badge st-badge-status">' + esc(rec.status) + "</span>";
      html += "</div>";

      html += '<dl class="st-meta">';
      html += "<dt>Issue</dt><dd>" + esc(id) + "</dd>";
      if (rec.cluster) html += "<dt>Category</dt><dd>" + esc(rec.cluster) + "</dd>";
      html += "<dt>Assignees</dt><dd>";
      html += (rec.assignees && rec.assignees.length)
        ? rec.assignees.map(function (p) { return '<span class="st-assignee">@' + esc(p) + "</span>"; }).join("")
        : "<em>none</em>";
      html += "</dd></dl>";

      html += relSection("Depends on", rec.depends_on, true);
      html += relSection("Blocks", rec.blocks, true);
      html += relSection("Related", rec.related, false);

      // body_html is sanitized at generation time; inject as-is.
      if (rec.body_html) html += '<div class="st-body">' + rec.body_html + "</div>";
      if (rec.url) html += '<a class="st-gh" target="_blank" rel="noopener" href="' + esc(rec.url) + '">View on GitHub &rarr;</a>';

      panel.innerHTML = html;
    }

    // Map clickable issue nodes to their records.
    var issueNodes = [];
    [].forEach.call(svg.querySelectorAll("g.node"), function (g) {
      var t = g.querySelector("title");
      if (!t) return;
      var id = t.textContent;
      if (!data[id]) return; // synthetic headers, drafts: no record
      issueNodes.push({ g: g, id: id });
      g.addEventListener("click", function (e) {
        e.preventDefault();
        e.stopPropagation();
        if (stage._stMoved) return; // was a drag, not a click
        show(g, id);
      });
    });

    // Click empty canvas (not a node, not a drag) -> clear panel.
    stage.addEventListener("click", function (e) {
      if (stage._stMoved) return;
      if (e.target.closest && e.target.closest("g.node")) return;
      clear();
    });

    // --- toolbar filters: search by # and status, both dim non-matches ---
    var search = widget.querySelector(".st-search");
    var statusSel = widget.querySelector(".st-status-filter");

    if (statusSel) {
      var seen = {};
      issueNodes.forEach(function (n) {
        var s = data[n.id].status;
        if (s) seen[s] = true;
      });
      Object.keys(seen).sort().forEach(function (s) {
        var o = document.createElement("option");
        o.value = s; o.textContent = s;
        statusSel.appendChild(o);
      });
    }

    function applyFilter() {
      var q = ((search && search.value) || "").trim().replace(/^#/, "");
      var st = (statusSel && statusSel.value) || "";
      issueNodes.forEach(function (n) {
        var rec = data[n.id];
        var okNum = !q || String(rec.number).indexOf(q) !== -1;
        var okStatus = !st || rec.status === st;
        n.g.classList.toggle("st-dim", !(okNum && okStatus));
      });
    }
    if (search) search.addEventListener("input", applyFilter);
    if (statusSel) statusSel.addEventListener("change", applyFilter);
  }

  document.addEventListener("DOMContentLoaded", function () {
    [].forEach.call(document.querySelectorAll(".st-widget"), initWidget);
  });
})();
