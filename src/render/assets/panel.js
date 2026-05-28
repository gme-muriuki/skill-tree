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

  // Pick a black/white text color that's readable on a given GitHub
  // label hex background. Threshold tuned for GitHub's typical palette
  // (saturated, mid-brightness); luminance via the standard sRGB
  // perceived-brightness coefficients.
  function labelText(hex) {
    var r = parseInt(hex.substr(0, 2), 16);
    var g = parseInt(hex.substr(2, 2), 16);
    var b = parseInt(hex.substr(4, 2), 16);
    return (r * 0.299 + g * 0.587 + b * 0.114) > 150 ? "#1f2328" : "#ffffff";
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

    function centerOn(g) {
      // Pixel-space deltas via getBoundingClientRect — accounts for both
      // the inner SVG viewBox and our outer CSS transform without us
      // having to invert either.
      var sr = stage.getBoundingClientRect();
      var gr = g.getBoundingClientRect();
      tx += (sr.left + sr.width / 2) - (gr.left + gr.width / 2);
      ty += (sr.top + sr.height / 2) - (gr.top + gr.height / 2);
      apply();
    }

    return {
      fit: fit,
      zoomIn: function () { zoomCenter(1.3); },
      zoomOut: function () { zoomCenter(1 / 1.3); },
      centerOn: centerOn,
    };
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
    var focusId = null; // currently-focused NodeId; drives neighborhood dim

    var zoom = setupZoom(stage, svg);
    buildToolbar(stage, zoom);

    function clear() {
      if (selected) selected.classList.remove("st-selected");
      selected = null;
      focusId = null;
      panel.innerHTML = emptyHTML;
      applyDim();
    }

    // relationship list; each neighbor resolved via the map. `marked`
    // shows a done/pending check (DEPENDS ON / BLOCKS); without it the
    // list is neutral (RELATED — decorative cross-refs/see-also). A
    // neighbor is clickable iff it has both a record AND an SVG node on
    // the board (ghost cross-refs render as plain text).
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
        var clickable = !!(r && byId[rid]);
        var attrs = clickable
          ? ' class="st-rel-item st-rel-clickable" role="button" tabindex="0" data-st-id="' + esc(rid) + '"'
          : ' class="st-rel-item"';
        return '<li' + attrs + ">" + mark + "<span>" + esc(title) + "</span></li>";
      }).join("");
      return '<div class="st-rel"><div class="st-rel-label">' + label +
             '</div><ul class="st-rel-list">' + items + "</ul></div>";
    }

    function show(g, id, recenter) {
      var rec = data[id];
      if (!rec) return;

      if (selected) selected.classList.remove("st-selected");
      selected = g;
      g.classList.add("st-selected");
      focusId = id;
      if (recenter) zoom.centerOn(g);
      applyDim();

      var html = '<h2 class="st-title">' + esc(rec.title) + "</h2>";

      html += '<div class="st-badges">';
      if (rec.state === "OPEN") html += '<span class="st-badge st-badge-open">OPEN</span>';
      else if (rec.state === "CLOSED") html += '<span class="st-badge st-badge-closed">CLOSED</span>';
      else if (rec.state) html += '<span class="st-badge st-badge-status">' + esc(rec.state) + "</span>";
      if (rec.status) html += '<span class="st-badge st-badge-status">' + esc(rec.status) + "</span>";
      html += "</div>";

      // Dependency progress bar: counts upstream blockers + sub-issues
      // (depends_on) that are no longer OPEN. Most useful for tracking
      // issues whose sub-issues dominate the list, but informative for
      // any node with blockers.
      if (rec.depends_on && rec.depends_on.length) {
        var total = rec.depends_on.length;
        var done = 0;
        rec.depends_on.forEach(function (rid) {
          var r = data[rid];
          if (r && r.state && r.state !== "OPEN") done++;
        });
        var pct = Math.round((done / total) * 100);
        html += '<div class="st-progress">' +
          '<div class="st-progress-label">' + done + ' of ' + total + ' upstream done</div>' +
          '<div class="st-progress-bar"><div class="st-progress-fill" style="width: ' + pct + '%"></div></div>' +
        '</div>';
      }

      html += '<dl class="st-meta">';
      html += "<dt>Issue</dt><dd>" + esc(id) + "</dd>";
      if (rec.cluster) html += "<dt>Category</dt><dd>" + esc(rec.cluster) + "</dd>";
      html += "<dt>Assignees</dt><dd>";
      html += (rec.assignees && rec.assignees.length)
        ? rec.assignees.map(function (p) { return '<span class="st-assignee">@' + esc(p) + "</span>"; }).join("")
        : "<em>none</em>";
      html += "</dd>";
      if (rec.labels && rec.labels.length) {
        html += "<dt>Labels</dt><dd>";
        html += rec.labels.map(function (l) {
          return '<span class="st-label" style="background:#' + esc(l.color) +
            ';color:' + labelText(l.color) + '">' + esc(l.name) + "</span>";
        }).join("");
        html += "</dd>";
      }
      html += "</dl>";

      html += relSection("Depends on", rec.depends_on, true);
      html += relSection("Blocks", rec.blocks, true);
      html += relSection("Related", rec.related, false);

      // body_html is sanitized at generation time; inject as-is. Wrapped
      // in a collapsible container so RFC-length bodies don't crowd the
      // panel (collapsed by default, "Show more" expands).
      if (rec.body_html) {
        html += '<div class="st-body-wrap st-collapsed">' +
          '<div class="st-body">' + rec.body_html + "</div>" +
          '<button type="button" class="st-body-toggle">Show more</button>' +
        "</div>";
      }
      if (rec.url) html += '<a class="st-gh" target="_blank" rel="noopener" href="' + esc(rec.url) + '">View on GitHub &rarr;</a>';

      panel.innerHTML = html;
    }

    // Index every clickable node (issues, PRs, drafts). Drafts have no
    // record so the click handler is only wired when one exists; the
    // filter operates on all of them so a draft titled "Foo" dims when
    // the user searches for "bar". `byId` lets the panel's relationship
    // rows look up an SVG node from a NodeId for click-to-navigate.
    var nodes = [];
    var byId = {};
    [].forEach.call(svg.querySelectorAll("g.node"), function (g) {
      var t = g.querySelector("title");
      if (!t) return;
      var id = t.textContent;
      var rec = data[id] || null;
      // Visible label = concatenated <text> children (graphviz emits one
      // per wrapped line). Lower-cased once for substring search.
      var label = [].map.call(g.querySelectorAll("text"), function (te) {
        return te.textContent || "";
      }).join(" ").toLowerCase();
      nodes.push({ g: g, id: id, rec: rec, label: label });
      byId[id] = g;
      if (rec) {
        g.addEventListener("click", function (e) {
          e.preventDefault();
          e.stopPropagation();
          if (stage._stMoved) return; // was a drag, not a click
          show(g, id, false);
        });
      }
    });

    // Index every edge so focus-mode can highlight 1-hop neighborhood
    // edges and dim the rest. Edge <title> is `srcId->tgtId` (graphviz
    // emits each directed edge that way; no NodeId format contains the
    // literal `->`).
    var edges = [];
    [].forEach.call(svg.querySelectorAll("g.edge"), function (g) {
      var t = g.querySelector("title");
      if (!t) return;
      var parts = t.textContent.split("->");
      if (parts.length !== 2) return;
      edges.push({ from: parts[0], to: parts[1], g: g });
    });

    // Synthetic project-root and cluster-header nodes (id prefix `__`) are
    // structural skeleton; never dim or highlight them by focus.
    function isStructural(id) { return id.indexOf("__") === 0; }

    // Navigate to a neighbor clicked from the panel relationship list.
    // Re-centers the SVG so the target lands in view even if it was
    // scrolled off the visible region.
    function navigate(id) {
      var g = byId[id];
      if (g && data[id]) show(g, id, true);
    }

    panel.addEventListener("click", function (e) {
      var toggle = e.target.closest && e.target.closest(".st-body-toggle");
      if (toggle) {
        var wrap = toggle.closest(".st-body-wrap");
        if (wrap) {
          var collapsed = wrap.classList.toggle("st-collapsed");
          toggle.textContent = collapsed ? "Show more" : "Show less";
        }
        return;
      }
      var item = e.target.closest && e.target.closest(".st-rel-clickable");
      if (!item) return;
      navigate(item.getAttribute("data-st-id"));
    });
    panel.addEventListener("keydown", function (e) {
      if (e.key !== "Enter" && e.key !== " ") return;
      var item = e.target.closest && e.target.closest(".st-rel-clickable");
      if (!item) return;
      e.preventDefault();
      navigate(item.getAttribute("data-st-id"));
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
      nodes.forEach(function (n) {
        var s = n.rec && n.rec.status;
        if (s) seen[s] = true;
      });
      Object.keys(seen).sort().forEach(function (s) {
        var o = document.createElement("option");
        o.value = s; o.textContent = s;
        statusSel.appendChild(o);
      });
    }

    // Dim state has two orthogonal sources: filter (search + status) and
    // focus (selected node's 1-hop neighborhood). A node is lit only if
    // BOTH say lit. Structural nodes (project root, cluster headers) are
    // skipped by focus dimming so the tree skeleton stays visible.
    function applyDim() {
      var q = ((search && search.value) || "").trim().toLowerCase().replace(/^#/, "");
      var st = (statusSel && statusSel.value) || "";
      var nbr = null;
      if (focusId && data[focusId]) {
        var rec = data[focusId];
        nbr = {};
        nbr[focusId] = true;
        ["depends_on", "blocks", "related"].forEach(function (k) {
          (rec[k] || []).forEach(function (rid) { nbr[rid] = true; });
        });
      }
      nodes.forEach(function (n) {
        var okQ = !q || n.label.indexOf(q) !== -1;
        var status = n.rec && n.rec.status;
        var okStatus = !st || status === st;
        var okFocus = !nbr || isStructural(n.id) || nbr[n.id];
        n.g.classList.toggle("st-dim", !(okQ && okStatus && okFocus));
      });
      edges.forEach(function (e) {
        var structural = isStructural(e.from) || isStructural(e.to);
        var lit = !nbr ||
          structural ||
          (e.from === focusId && nbr[e.to]) ||
          (e.to === focusId && nbr[e.from]);
        e.g.classList.toggle("st-dim", !!nbr && !lit);
        e.g.classList.toggle(
          "st-edge-focus",
          !!nbr && !structural && lit
        );
      });
    }
    if (search) search.addEventListener("input", applyDim);
    if (statusSel) statusSel.addEventListener("change", applyDim);
  }

  document.addEventListener("DOMContentLoaded", function () {
    [].forEach.call(document.querySelectorAll(".st-widget"), initWidget);
  });
})();
