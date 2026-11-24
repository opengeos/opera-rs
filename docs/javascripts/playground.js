// Interactive browser playground for opera-rs compiled to WebAssembly (opera-js),
// paired with live public NASA CMR and titiler-cmr endpoints (no Earthdata login).
(function () {
  "use strict";

  const SCRIPT_SRC = document.currentScript ? document.currentScript.src : window.location.href;
  const SITE_ROOT = new URL("../", SCRIPT_SRC);
  const WASM_URL = new URL("wasm/opera_js.js", SITE_ROOT).href;
  const MAPLIBRE_CSS = "https://unpkg.com/maplibre-gl@5.1.0/dist/maplibre-gl.css";
  const MAPLIBRE_JS = "https://unpkg.com/maplibre-gl@5.1.0/dist/maplibre-gl.js";

  const EXPLORER_PRESETS = {
    tulare: {
      product: "OPERA_L3_DSWX-HLS_V1",
      band: "B01_WTR",
      start: "2023-04-20",
      end: "2023-04-30",
      bbox: "-119.95,35.85,-119.45,36.25",
      waterOnly: true,
      expression: "",
    },
    valencia: {
      product: "OPERA_L3_DSWX-S1_V1",
      band: "B01_WTR",
      start: "2024-10-27",
      end: "2024-11-05",
      bbox: "-0.70,39.15,-0.25,39.55",
      waterOnly: true,
      expression: "",
    },
    gsl: {
      product: "OPERA_L3_DSWX-HLS_V1",
      band: "B01_WTR",
      start: "2023-06-01",
      end: "2023-06-15",
      bbox: "-113.15,40.65,-111.90,41.70",
      waterOnly: true,
      expression: "",
    },
    "la-fire": {
      product: "OPERA_L3_DIST-ALERT-HLS_V1",
      band: "VEG-DIST-STATUS",
      start: "2025-01-10",
      end: "2025-01-25",
      bbox: "-118.70,34.00,-118.00,34.30",
      waterOnly: false,
      expression: "",
    },
    houston: {
      product: "OPERA_L2_RTC-S1_V1",
      band: "VV",
      start: "2024-07-05",
      end: "2024-07-15",
      bbox: "-95.70,29.55,-95.10,29.95",
      waterOnly: false,
      expression: "10*log10(b1)",
    },
  };

  const SAMPLE_CSLC_LINES = [
    "OPERA_L2_CSLC-S1_T087-185683-IW2_20230322T161649Z_20240504T185235Z_S1A_VV_v1.1.h5",
    "OPERA_L2_CSLC-S1_T087-185683-IW2_20230403T161649Z_20240504T190112Z_S1A_VV_v1.1.h5",
    "OPERA_L2_CSLC-S1_T087-185684-IW2_20230322T161652Z_20240504T185240Z_S1A_VV_v1.1.h5",
    "https://datapool.asf.alaska.edu/CSLC/OPERA_L2_COMPRESSED-CSLC-S1_F11114_T087-185683-IW2_20230101T161649Z_20230101T161649Z_20230322T161649Z_20240505T000000Z_VV_v1.0.h5",
  ];

  const SAMPLE_DISP_LINES = [
    "https://datapool.asf.alaska.edu/DISP/OPERA-S1/OPERA_L3_DISP-S1_IW_F11114_VV_20160705T140755Z_20230418T140812Z_v1.0_20241219T231545Z.nc",
    "https://datapool.asf.alaska.edu/DISP/OPERA-S1/OPERA_L3_DISP-S1_IW_F11114_VV_20160705T140755Z_20230418T140812Z_v1.0_20250105T021015Z.nc",
    "https://datapool.asf.alaska.edu/DISP/OPERA-S1/OPERA_L3_DISP-S1_IW_F11114_VV_20160705T140755Z_20230430T140812Z_v1.0_20241220T112000Z.nc",
    "s3://asf-ngap2w-p-s1-disp/OPERA_L3_DISP-S1_IW_F11114_VV_20160705T140755Z_20230512T140812Z_v1.0_20241221T080510Z.nc",
  ];

  let wasmPromise = null;
  let maplibrePromise = null;
  let wasmMod = null;
  let catalog = [];
  let map = null;
  let mapReady = false;
  let lastSearch = null;
  let pinnedGranuleUr = null;
  let lastTitiler = null;
  let lastDispResult = null;
  let activeAbort = null;
  let dispAbort = null;

  // Missing-data interactive state
  let mdState = {
    burstIds: [],
    dates: [],
    missing: new Set(),
    options: [],
    selectedIdx: 0,
  };

  const $ = (id) => document.getElementById(id);

  function loadWasm() {
    if (!wasmPromise) {
      wasmPromise = import(WASM_URL).then(async (m) => {
        await m.default();
        return m;
      });
    }
    return wasmPromise;
  }

  function loadMapLibre() {
    if (window.maplibregl) return Promise.resolve(window.maplibregl);
    if (!maplibrePromise) {
      maplibrePromise = new Promise((resolve, reject) => {
        if (!document.querySelector(`link[href="${MAPLIBRE_CSS}"]`)) {
          const link = document.createElement("link");
          link.rel = "stylesheet";
          link.href = MAPLIBRE_CSS;
          document.head.appendChild(link);
        }
        const script = document.createElement("script");
        script.src = MAPLIBRE_JS;
        script.async = true;
        script.onload = () => resolve(window.maplibregl);
        script.onerror = () => reject(new Error("Failed to load MapLibre GL JS from CDN"));
        document.head.appendChild(script);
      });
    }
    return maplibrePromise;
  }

  function setStatus(text, isError) {
    const el = $("op-status");
    if (!el) return;
    el.textContent = text;
    el.classList.toggle("op-error", !!isError);
  }

  function errMsg(e) {
    return e && e.message ? e.message : String(e);
  }

  function clearEl(el) {
    while (el && el.firstChild) el.removeChild(el.firstChild);
  }

  function addStat(container, value, label) {
    const div = document.createElement("div");
    div.className = "op-stat";
    const strong = document.createElement("strong");
    strong.textContent = String(value);
    const span = document.createElement("span");
    span.textContent = label;
    div.appendChild(strong);
    div.appendChild(span);
    container.appendChild(div);
  }

  function downloadText(filename, content, mime) {
    const blob = new Blob([content], { type: mime || "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  function parseBBoxInput(str) {
    const parts = String(str || "")
      .split(",")
      .map((s) => Number(s.trim()));
    if (parts.length !== 4 || parts.some((n) => !Number.isFinite(n))) {
      throw new Error("BBox must be four comma-separated numbers: west,south,east,north");
    }
    const [w, s, e, n] = parts;
    if (w >= e || s >= n || w < -180 || e > 180 || s < -90 || n > 90) {
      throw new Error("Invalid WGS 84 bbox bounds");
    }
    return [w, s, e, n];
  }

  function bboxToPolygonGeoJson([w, s, e, n]) {
    return {
      type: "Feature",
      properties: {},
      geometry: {
        type: "Polygon",
        coordinates: [
          [
            [w, s],
            [e, s],
            [e, n],
            [w, n],
            [w, s],
          ],
        ],
      },
    };
  }

  function currentProductObj() {
    const shortName = $("op-product") ? $("op-product").value : "OPERA_L3_DSWX-HLS_V1";
    return catalog.find((p) => p.short_name === shortName) || catalog[0];
  }

  function populatePresetsDropdown() {
    if (!wasmMod || !$("op-preset-expr")) return;
    const prod = currentProductObj();
    const band = ($("op-band").value || "").trim() || (prod.default_bands[0] || "");
    const sel = $("op-preset-expr");
    clearEl(sel);
    const defOpt = document.createElement("option");
    defOpt.value = "";
    defOpt.textContent = "Default band styling";
    sel.appendChild(defOpt);
    try {
      const presets = JSON.parse(wasmMod.expression_presets_json(prod.short_name, band));
      for (const p of presets) {
        const opt = document.createElement("option");
        opt.value = p.expression || "";
        opt.dataset.rescale = p.rescale || "";
        opt.textContent = `${p.label}: ${p.expression}`;
        sel.appendChild(opt);
      }
    } catch (_) {
      // Ignore if product/band has no presets
    }
  }

  function updateLegend() {
    if (!wasmMod || !$("op-legend")) return;
    const prod = currentProductObj();
    const band = ($("op-band").value || "").trim() || null;
    const waterOnly = $("op-water-only").checked;
    const el = $("op-legend");
    clearEl(el);
    try {
      const entries = JSON.parse(wasmMod.legend_entries_json(prod.short_name, band, waterOnly));
      for (const entry of entries) {
        const item = document.createElement("span");
        item.className = "op-legend-item";
        const sw = document.createElement("i");
        sw.style.background = entry.css;
        item.appendChild(sw);
        item.appendChild(document.createTextNode(entry.label));
        el.appendChild(item);
      }
    } catch (_) {
      // Non-categorical product
    }
  }

  function updateEquivalentCommands() {
    const codeEl = $("op-equiv-cmd");
    if (!codeEl) return;
    const prod = currentProductObj();
    const alias = (prod && prod.aliases && prod.aliases[0]) || (prod ? prod.short_name : "dswx-hls");
    const bboxStr = ($("op-bbox").value || "").trim();
    const start = $("op-start").value;
    const end = $("op-end").value;
    const band = ($("op-band").value || "").trim();
    const waterOnly = $("op-water-only").checked;
    const expr = $("op-preset-expr").value;

    const cliParts = [
      `opera search --product ${alias} --bbox ${bboxStr}`,
      start ? `--start ${start}` : "",
      end ? `--end ${end}` : "",
      "--json",
    ].filter(Boolean);

    const previewParts = [
      `opera preview --product ${alias} --bbox ${bboxStr}`,
      start ? `--start ${start}` : "",
      end ? `--end ${end}` : "",
      band ? `--band ${band}` : "",
      waterOnly ? "--water-only" : "",
      expr ? `--expression "${expr}"` : "",
      "-o preview.png",
    ].filter(Boolean);

    const pyCode = [
      "import opera_rs",
      `bbox = [${bboxStr}]`,
      `res = opera_rs.search("${alias}", bbox=bbox, start="${start}", end="${end}")`,
      `st = opera_rs.stats(bbox, product="${alias}", start="${start}", end="${end}")`,
    ].join("\n");

    codeEl.textContent = `# CLI\n${cliParts.join(" ")}\n${previewParts.join(" ")}\n\n# Python\n${pyCode}`;
  }

  async function ensureMap() {
    if (map) return map;
    const container = $("op-map");
    if (!container) return null;
    const maplibregl = await loadMapLibre();
    map = new maplibregl.Map({
      container,
      style: {
        version: 8,
        sources: {
          osm: {
            type: "raster",
            tiles: ["https://tile.openstreetmap.org/{z}/{x}/{y}.png"],
            tileSize: 256,
            attribution: "&copy; OpenStreetMap contributors",
          },
        },
        layers: [{ id: "osm-base", type: "raster", source: "osm" }],
      },
      center: [-119.7, 36.05],
      zoom: 9,
    });
    map.addControl(new maplibregl.NavigationControl(), "top-right");
    map.addControl(new maplibregl.FullscreenControl(), "top-right");
    map.addControl(new maplibregl.ScaleControl({ maxWidth: 120, unit: "metric" }), "bottom-left");
    await new Promise((resolve) => {
      if (map.loaded()) resolve();
      else map.once("load", resolve);
    });
    mapReady = true;

    map.on("click", async (ev) => {
      if (!lastTitiler || !wasmMod || !lastSearch || !lastSearch.concept_id) return;
      const { lng, lat } = ev.lngLat;
      const box = $("op-point-box");
      if (!box) return;
      box.hidden = false;
      box.textContent = `Sampling (${lng.toFixed(4)}, ${lat.toFixed(4)}) via titiler-cmr...`;
      try {
        const prod = currentProductObj();
        const band = ($("op-band").value || "").trim() || null;
        const expr = $("op-preset-expr").value || null;
        const temporal = pinnedGranuleUr ? null : buildTemporalRange();
        const url = wasmMod.build_point_url_js(
          lastSearch.concept_id,
          prod.short_name,
          prod.backend,
          pinnedGranuleUr,
          temporal,
          band,
          expr,
          lng,
          lat,
          null
        );
        const resp = await fetch(url);
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        const text = await resp.text();
        const parsed = JSON.parse(
          wasmMod.parse_point_response_json(text, prod.short_name, band)
        );
        const summary = (parsed.assets || [])
          .map((a) => {
            const vals = (a.values || [])
              .map((v, idx) => {
                const lbl = a.class_labels && a.class_labels[idx];
                const vStr = v === null || v === undefined ? "nodata" : Number(v).toFixed(3).replace(/\.000$/, "");
                return lbl ? `${vStr} (${lbl})` : vStr;
              })
              .join(", ");
            return `${a.name}: ${vals || "nodata"}`;
          })
          .join(" | ");
        box.textContent = `Point (${lng.toFixed(4)}, ${lat.toFixed(4)}): ${summary || "no valid pixels"}`;
      } catch (e) {
        box.textContent = `Point (${lng.toFixed(4)}, ${lat.toFixed(4)}): ${errMsg(e)}`;
      }
    });

    return map;
  }

  function buildTemporalRange() {
    const start = $("op-start").value;
    const end = $("op-end").value;
    if (start && end) return `${start}T00:00:00Z/${end}T23:59:59Z`;
    if (start) return `${start}T00:00:00Z/..`;
    if (end) return `../${end}T23:59:59Z`;
    return null;
  }

  function updateMapOverlay(bbox, fc, tileUrlTemplate) {
    if (!map || !mapReady) return;
    if (map.getLayer("opera-tiles-layer")) map.removeLayer("opera-tiles-layer");
    if (map.getSource("opera-tiles")) map.removeSource("opera-tiles");
    if (map.getLayer("opera-footprints-line")) map.removeLayer("opera-footprints-line");
    if (map.getLayer("opera-footprints-fill")) map.removeLayer("opera-footprints-fill");
    if (map.getSource("opera-footprints")) map.removeSource("opera-footprints");
    if (map.getLayer("opera-aoi-line")) map.removeLayer("opera-aoi-line");
    if (map.getSource("opera-aoi")) map.removeSource("opera-aoi");

    if (tileUrlTemplate) {
      map.addSource("opera-tiles", {
        type: "raster",
        tiles: [tileUrlTemplate],
        tileSize: 256,
      });
      map.addLayer({
        id: "opera-tiles-layer",
        type: "raster",
        source: "opera-tiles",
        paint: { "raster-opacity": parseFloat($("op-opacity").value || "0.85") },
      });
    }

    if (fc && fc.features && fc.features.length) {
      map.addSource("opera-footprints", { type: "geojson", data: fc });
      map.addLayer({
        id: "opera-footprints-fill",
        type: "fill",
        source: "opera-footprints",
        paint: { "fill-color": "#0d9488", "fill-opacity": 0.05 },
      });
      map.addLayer({
        id: "opera-footprints-line",
        type: "line",
        source: "opera-footprints",
        paint: { "line-color": "#0d9488", "line-width": 1.6 },
      });
    }

    map.addSource("opera-aoi", { type: "geojson", data: bboxToPolygonGeoJson(bbox) });
    map.addLayer({
      id: "opera-aoi-line",
      type: "line",
      source: "opera-aoi",
      paint: { "line-color": "#dc2626", "line-width": 2.2, "line-dasharray": [4, 2] },
    });

    map.fitBounds(
      [
        [bbox[0], bbox[1]],
        [bbox[2], bbox[3]],
      ],
      { padding: 36, maxZoom: 12, duration: 450 }
    );
  }

  function applyTileLayerFromState() {
    if (!wasmMod || !lastSearch || !lastSearch.concept_id) return;
    const prod = currentProductObj();
    const band = ($("op-band").value || "").trim() || null;
    const exprSel = $("op-preset-expr");
    const expr = exprSel.value || null;
    const opt = exprSel.selectedOptions && exprSel.selectedOptions[0];
    const rescale = opt && opt.dataset ? opt.dataset.rescale || null : null;
    const waterOnly = $("op-water-only").checked;
    const temporal = pinnedGranuleUr ? null : buildTemporalRange();

    lastTitiler = JSON.parse(
      wasmMod.build_titiler_urls_json(
        lastSearch.concept_id,
        prod.short_name,
        prod.backend,
        pinnedGranuleUr,
        temporal,
        band,
        waterOnly,
        expr,
        rescale,
        null,
        256
      )
    );
    const bbox = parseBBoxInput($("op-bbox").value);
    updateMapOverlay(bbox, lastSearch.feature_collection, lastTitiler.tile_url_template);
    updateLegend();
    const unpinBtn = $("op-unpin-btn");
    if (unpinBtn) unpinBtn.hidden = !pinnedGranuleUr;
    const hint = $("op-map-hint");
    if (hint) {
      hint.textContent = pinnedGranuleUr
        ? `Pinned granule: ${pinnedGranuleUr}. Click map to sample pixel values.`
        : `Showing ${prod.short_title} (${band || "default"}) composite. Click map to sample pixel values.`;
    }
  }

  async function runExplorerSearch() {
    if (!wasmMod) return;
    if (activeAbort) activeAbort.abort();
    activeAbort = new AbortController();
    const signal = activeAbort.signal;

    const btn = $("op-search-btn");
    btn.disabled = true;
    pinnedGranuleUr = null;
    $("op-point-box").hidden = true;
    updateEquivalentCommands();

    try {
      const prod = currentProductObj();
      const bbox = parseBBoxInput($("op-bbox").value);
      const start = $("op-start").value || null;
      const end = $("op-end").value || null;
      const dedup = $("op-dedup").checked;

      setStatus(`Querying NASA CMR for ${prod.short_name}...`);
      const cmrUrl = wasmMod.build_cmr_search_url_js(
        prod.short_name,
        bbox[0],
        bbox[1],
        bbox[2],
        bbox[3],
        start,
        end,
        50
      );
      const t0 = performance.now();
      const resp = await fetch(cmrUrl, { signal });
      if (!resp.ok) throw new Error(`CMR returned HTTP ${resp.status}`);
      const body = await resp.text();
      const tWasm0 = performance.now();
      lastSearch = JSON.parse(wasmMod.parse_cmr_page_json(body, prod.short_name, dedup, false));
      const wasmMs = performance.now() - tWasm0;
      const totalMs = performance.now() - t0;

      const summary = $("op-explorer-summary");
      clearEl(summary);
      addStat(summary, lastSearch.hits.toLocaleString(), "CMR total hits");
      addStat(summary, lastSearch.granules.length, "Granules loaded");
      addStat(summary, lastSearch.dates.length, "Distinct dates");
      addStat(summary, `${wasmMs.toFixed(1)} ms`, "WASM UMM-G parse");
      summary.hidden = false;

      // Populate granules table
      const tbody = $("op-granules-tbody");
      clearEl(tbody);
      for (const g of lastSearch.granules) {
        const tr = document.createElement("tr");
        tr.className = "op-clickable";
        tr.dataset.ur = g.id;
        for (const val of [g.date, g.time, g.tile, (g.bands || []).slice(0, 4).join(", "), g.id]) {
          const td = document.createElement("td");
          td.textContent = val || "-";
          tr.appendChild(td);
        }
        tr.addEventListener("click", () => {
          pinnedGranuleUr = g.id;
          for (const row of tbody.querySelectorAll("tr")) {
            row.classList.toggle("op-row-selected", row.dataset.ur === pinnedGranuleUr);
          }
          applyTileLayerFromState();
        });
        tbody.appendChild(tr);
      }
      $("op-granules-section").hidden = false;

      await ensureMap();
      if (lastSearch.concept_id) {
        applyTileLayerFromState();
        $("op-stats-btn").disabled = false;
        setStatus(
          `Loaded ${lastSearch.granules.length} granules (${lastSearch.hits} CMR hits) in ${totalMs.toFixed(0)} ms (WASM parse: ${wasmMs.toFixed(1)} ms).`
        );
      } else {
        updateMapOverlay(bbox, lastSearch.feature_collection, null);
        $("op-stats-btn").disabled = true;
        setStatus(`Found ${lastSearch.granules.length} granules, but no collection concept_id for tiling.`);
      }
    } catch (e) {
      if (e && e.name === "AbortError") return;
      setStatus(`Search failed: ${errMsg(e)}`, true);
    } finally {
      btn.disabled = false;
    }
  }

  function renderStatsChart(statsObj) {
    const canvas = $("op-stats-chart");
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    const w = canvas.width;
    const h = canvas.height;
    ctx.clearRect(0, 0, w, h);

    const bands = Object.entries(statsObj.bands || {});
    if (!bands.length) {
      ctx.fillStyle = "#6b7280";
      ctx.font = "13px sans-serif";
      ctx.fillText("No valid pixels returned for this AOI/date range.", 18, 40);
      return;
    }

    const [, bandStats] = bands[0];
    const breakdown = bandStats.class_breakdown || [];
    const tbody = $("op-stats-tbody");
    clearEl(tbody);

    const fgColor = getComputedStyle(document.body).color || "#374151";

    if (breakdown.length) {
      const top = breakdown.slice(0, 7);
      const maxPct = Math.max(...top.map((c) => c.percent || 0), 1);
      const rowH = Math.min(28, Math.floor((h - 24) / top.length));
      const labelW = 156;
      const barMaxW = w - labelW - 118;
      const palette = ["#0284c7", "#38bdf8", "#0d9488", "#94a3b8", "#f59e0b", "#64748b", "#a855f7"];

      top.forEach((cls, idx) => {
        const y = 14 + idx * rowH;
        const pct = Number(cls.percent || 0);
        const area = Number(cls.area_km2 || 0);
        const pxCount = Math.round(Number(cls.pixel_count || 0));
        const name = cls.label || `Class ${cls.class_value}`;

        ctx.fillStyle = fgColor;
        ctx.font = "11px sans-serif";
        ctx.textAlign = "right";
        ctx.fillText(name.slice(0, 22), labelW - 8, y + 14);

        const bw = Math.max(2, (pct / maxPct) * barMaxW);
        ctx.fillStyle = palette[idx % palette.length];
        ctx.fillRect(labelW, y + 3, bw, rowH - 8);

        ctx.fillStyle = fgColor;
        ctx.textAlign = "left";
        ctx.fillText(`${area.toFixed(1)} km2 (${pct.toFixed(1)}%)`, labelW + bw + 6, y + 14);

        const tr = document.createElement("tr");
        for (const v of [name, pxCount.toLocaleString(), `${pct.toFixed(2)}%`, area.toFixed(2)]) {
          const td = document.createElement("td");
          td.textContent = v;
          tr.appendChild(td);
        }
        tbody.appendChild(tr);
      });
      $("op-stats-table-wrap").hidden = false;
    } else {
      // Continuous raster stats summary
      const rows = [
        ["Min", bandStats.min],
        ["Mean", bandStats.mean],
        ["Median", bandStats.median],
        ["Max", bandStats.max],
        ["Std Dev", bandStats.std],
        ["Valid %", bandStats.valid_percent],
      ].filter(([, v]) => v !== null && v !== undefined);
      ctx.fillStyle = fgColor;
      ctx.font = "13px monospace";
      rows.forEach(([k, v], idx) => {
        ctx.fillText(`${k.padEnd(10)}: ${Number(v).toFixed(3)}`, 24, 34 + idx * 24);
        const tr = document.createElement("tr");
        for (const cell of [k, "-", "-", Number(v).toFixed(3)]) {
          const td = document.createElement("td");
          td.textContent = cell;
          tr.appendChild(td);
        }
        tbody.appendChild(tr);
      });
      $("op-stats-table-wrap").hidden = false;
    }
  }

  async function computeZonalStats() {
    if (!wasmMod || !lastTitiler) return;
    const btn = $("op-stats-btn");
    btn.disabled = true;
    $("op-stats-note").textContent = "Computing live zonal statistics via titiler-cmr...";
    try {
      const bbox = parseBBoxInput($("op-bbox").value);
      const prod = currentProductObj();
      const band = ($("op-band").value || "").trim() || null;
      const t0 = performance.now();
      const resp = await fetch(lastTitiler.statistics_url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(bboxToPolygonGeoJson(bbox)),
      });
      if (!resp.ok) throw new Error(`titiler-cmr returned HTTP ${resp.status}`);
      const text = await resp.text();
      const parsed = JSON.parse(
        wasmMod.parse_statistics_response_json(text, prod.short_name, band, lastTitiler.categorical)
      );
      renderStatsChart(parsed);
      $("op-stats-note").textContent = `Computed in ${(performance.now() - t0).toFixed(0)} ms (30 m pixel areas calculated in Rust WASM).`;
    } catch (e) {
      $("op-stats-note").textContent = `Statistics failed: ${errMsg(e)}`;
    } finally {
      btn.disabled = false;
    }
  }

  async function geocodePlace() {
    const q = ($("op-place").value || "").trim();
    if (!q) return;
    setStatus(`Geocoding "${q}" via Nominatim...`);
    try {
      const url = `https://nominatim.openstreetmap.org/search?q=${encodeURIComponent(q)}&format=jsonv2&limit=1`;
      const resp = await fetch(url);
      if (!resp.ok) throw new Error(`Nominatim HTTP ${resp.status}`);
      const items = await resp.json();
      if (!items.length || !items[0].boundingbox) throw new Error("No matching place found");
      const [s, n, w, e] = items[0].boundingbox.map(Number);
      $("op-bbox").value = `${w.toFixed(4)},${s.toFixed(4)},${e.toFixed(4)},${n.toFixed(4)}`;
      updateEquivalentCommands();
      setStatus(`Geocoded "${items[0].display_name}". Click Search & Render to query CMR.`);
    } catch (e) {
      setStatus(`Geocode failed: ${errMsg(e)}`, true);
    }
  }

  // ---- TAB 2: BURST & DISP-S1 PARSER (WASM) ----
  function runBurstParser() {
    if (!wasmMod) return;
    const lines = ($("op-burst-input").value || "")
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter((s) => s && !s.startsWith("#"));
    if (!lines.length) return;

    const t0 = performance.now();
    const tbody = $("op-burst-tbody");
    clearEl(tbody);

    const cslcLines = [];
    const dispLines = [];
    let okCount = 0;

    for (const line of lines) {
      const tr = document.createElement("tr");
      let cells = [];
      try {
        if (line.includes("DISP-S1")) {
          const d = JSON.parse(wasmMod.parse_disp_filename_json(line));
          dispLines.push(line);
          cells = [
            "DISP-S1",
            `F${String(d.frame_id).padStart(5, "0")}`,
            `${d.reference_datetime.slice(0, 10)} -> ${d.secondary_datetime.slice(0, 10)}`,
            `${d.polarization} / v${d.version}`,
            `gen ${d.generation_datetime.slice(0, 10)} (${d.acquisition_mode})`,
          ];
          okCount++;
        } else {
          const c = JSON.parse(wasmMod.parse_cslc_filename_json(line));
          cslcLines.push(line);
          const dates = JSON.parse(wasmMod.get_dates_json(line));
          const kind = c.is_compressed ? "COMPRESSED-CSLC" : "CSLC-S1";
          cells = [
            kind,
            c.burst_id,
            dates.join(", "),
            `${c.polarization || "-"} / ${c.product_version || "-"}`,
            c.frame_id ? `frame F${c.frame_id}` : c.sensor || "burst match",
          ];
          okCount++;
        }
      } catch (_) {
        try {
          const b = JSON.parse(wasmMod.parse_burst_id_json(line));
          cslcLines.push(line);
          const dates = JSON.parse(wasmMod.get_dates_json(line));
          cells = [
            "Burst ID",
            b.normalized,
            dates.join(", ") || "-",
            "-",
            `official ${b.official} (T${b.track} IW${b.subswath})`,
          ];
          okCount++;
        } catch (err) {
          cells = ["Error", "-", "-", "-", errMsg(err)];
        }
      }
      for (const txt of cells) {
        const td = document.createElement("td");
        td.textContent = txt;
        tr.appendChild(td);
      }
      tbody.appendChild(tr);
    }

    const summary = $("op-burst-summary");
    clearEl(summary);
    addStat(summary, lines.length, "Input lines");
    addStat(summary, okCount, "Parsed cleanly");

    const groupsWrap = $("op-burst-groups-wrap");
    const groupsBody = $("op-burst-groups-tbody");
    clearEl(groupsBody);

    if (cslcLines.length) {
      const grouped = JSON.parse(wasmMod.group_by_burst_json(JSON.stringify(cslcLines)));
      const entries = Object.entries(grouped);
      addStat(summary, entries.length, "Distinct burst IDs");
      for (const [bid, files] of entries) {
        const tr = document.createElement("tr");
        for (const v of [bid, String(files.length), files.map((f) => f.split("/").pop()).join(", ")]) {
          const td = document.createElement("td");
          td.textContent = v;
          tr.appendChild(td);
        }
        groupsBody.appendChild(tr);
      }
      groupsWrap.hidden = false;
    } else {
      groupsWrap.hidden = true;
    }

    if (dispLines.length) {
      const dedup = JSON.parse(wasmMod.deduplicate_disp_filenames_json(JSON.stringify(dispLines)));
      addStat(summary, dedup.products.length, "Deduplicated DISP-S1");
      addStat(summary, dedup.duplicates_removed, "Superseded removed");
    }

    const elapsed = performance.now() - t0;
    $("op-parse-timing").textContent = `Parsed in ${elapsed.toFixed(2)} ms (100% client-side Rust WASM).`;
    summary.hidden = false;
    $("op-burst-results").hidden = false;
  }

  // ---- TAB 3: MISSING-DATA SUBSET OPTIMIZER (WASM) ----
  function loadMissingDataPreset(numBursts, numDates, missingPairs) {
    mdState.burstIds = Array.from(
      { length: numBursts },
      (_, i) => `t042_${String(88900 + i).padStart(6, "0")}_iw${(i % 3) + 1}`
    );
    mdState.dates = Array.from({ length: numDates }, (_, d) => {
      const yr = 2023 + Math.floor(d / 24);
      const m = Math.floor((d % 24) / 2) + 1;
      const day = (d % 2) * 12 + 1;
      return `${yr}-${String(m).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
    });
    mdState.missing = new Set(missingPairs.map(([b, d]) => `${b}:${d}`));
    mdState.selectedIdx = 0;
    runMissingDataOptimizer();
  }

  function runMissingDataOptimizer() {
    if (!wasmMod || !mdState.burstIds.length) return;
    const files = [];
    for (let bi = 0; bi < mdState.burstIds.length; bi++) {
      const bOfficial = mdState.burstIds[bi].toUpperCase().replaceAll("_", "-");
      for (let di = 0; di < mdState.dates.length; di++) {
        if (mdState.missing.has(`${bi}:${di}`)) continue;
        const dtCompact = mdState.dates[di].replaceAll("-", "");
        files.push(
          `OPERA_L2_CSLC-S1_${bOfficial}_${dtCompact}T140000Z_20240504T185235Z_S1A_VV_v1.1.h5`
        );
      }
    }

    const t0 = performance.now();
    mdState.options = JSON.parse(
      wasmMod.get_missing_data_options_json(JSON.stringify(files), 25)
    );
    const ms = performance.now() - t0;
    if (mdState.selectedIdx >= mdState.options.length) mdState.selectedIdx = 0;

    const best = mdState.options[mdState.selectedIdx];
    const summary = $("op-md-summary");
    clearEl(summary);
    addStat(summary, `${mdState.burstIds.length} x ${mdState.dates.length}`, "Full stack grid");
    addStat(summary, files.length, "Available CSLCs");
    addStat(summary, mdState.missing.size, "Missing gaps");
    if (best) {
      addStat(summary, `${best.num_burst_ids} x ${best.num_dates} (${best.total_num_bursts})`, "Selected option");
    }
    addStat(summary, `${ms.toFixed(2)} ms`, "WASM solve time");
    $("op-md-timing").textContent = `Solved ${mdState.options.length} ranked options in ${ms.toFixed(2)} ms`;

    const tbody = $("op-md-tbody");
    clearEl(tbody);
    mdState.options.forEach((opt, idx) => {
      const tr = document.createElement("tr");
      tr.className = "op-clickable" + (idx === mdState.selectedIdx ? " op-row-selected" : "");
      for (const v of [
        String(idx + 1),
        String(opt.num_dates),
        String(opt.num_burst_ids),
        String(opt.total_num_bursts),
        String(opt.excluded_bursts),
      ]) {
        const td = document.createElement("td");
        td.textContent = v;
        tr.appendChild(td);
      }
      tr.addEventListener("click", () => {
        mdState.selectedIdx = idx;
        for (const [rIdx, row] of Array.from(tbody.querySelectorAll("tr")).entries()) {
          row.classList.toggle("op-row-selected", rIdx === idx);
        }
        drawMissingDataMatrix();
      });
      tbody.appendChild(tr);
    });

    drawMissingDataMatrix();
  }

  function mdMatrixGeometry(canvas) {
    const padL = 112;
    const padT = 14;
    const padR = 12;
    const padB = 42;
    const cellW = (canvas.width - padL - padR) / Math.max(mdState.dates.length, 1);
    const cellH = (canvas.height - padT - padB) / Math.max(mdState.burstIds.length, 1);
    return { padL, padT, padR, padB, cellW, cellH };
  }

  function drawMissingDataMatrix() {
    const canvas = $("op-md-canvas");
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    ctx.clearRect(0, 0, canvas.width, canvas.height);

    const opt = mdState.options[mdState.selectedIdx];
    const selBursts = new Set(opt ? opt.burst_ids : []);
    const selDates = new Set(opt ? opt.dates : []);
    const { padL, padT, cellW, cellH } = mdMatrixGeometry(canvas);

    for (let bi = 0; bi < mdState.burstIds.length; bi++) {
      const b = mdState.burstIds[bi];
      for (let di = 0; di < mdState.dates.length; di++) {
        const dt = mdState.dates[di];
        const x = padL + di * cellW;
        const y = padT + bi * cellH;
        if (mdState.missing.has(`${bi}:${di}`)) {
          ctx.fillStyle = "#dc2626";
        } else if (selBursts.has(b) && selDates.has(dt)) {
          ctx.fillStyle = "#2a9d8f";
        } else {
          ctx.fillStyle = "#9ca3af";
        }
        ctx.fillRect(x + 1, y + 1, Math.max(2, cellW - 2), Math.max(2, cellH - 2));
      }
      if (mdState.burstIds.length <= 16 || bi % 2 === 0) {
        ctx.fillStyle = "#4b5563";
        ctx.font = "10px monospace";
        ctx.textAlign = "right";
        ctx.fillText(b, padL - 6, padT + bi * cellH + cellH * 0.72);
      }
    }

    const step = mdState.dates.length > 12 ? 3 : 1;
    ctx.fillStyle = "#4b5563";
    ctx.font = "10px monospace";
    ctx.textAlign = "center";
    for (let di = 0; di < mdState.dates.length; di += step) {
      const x = padL + di * cellW + cellW / 2;
      ctx.fillText(mdState.dates[di].slice(5), x, canvas.height - 16);
    }
  }

  // ---- TAB 4: LIVE DISP-S1 FRAME SEARCH ----
  async function runDispSearch() {
    if (!wasmMod) return;
    if (dispAbort) dispAbort.abort();
    dispAbort = new AbortController();

    const btn = $("op-disp-search-btn");
    btn.disabled = true;
    try {
      const frameId = parseInt($("op-disp-frame").value, 10);
      if (!Number.isFinite(frameId) || frameId < 1) {
        throw new Error("Enter a positive DISP-S1 frame ID (e.g. 11114)");
      }
      const ver = ($("op-disp-ver").value || "").trim() || null;
      const urlType = $("op-disp-url-type").value;
      const limit = parseInt($("op-disp-limit").value, 10) || 50;
      const dedup = $("op-disp-dedup").checked;

      setStatus(`Searching NASA CMR for OPERA_L3_DISP-S1 frame ${frameId}...`);
      const cmrUrl = wasmMod.build_disp_search_url_js(
        frameId,
        null,
        null,
        null,
        null,
        null,
        null,
        ver,
        limit
      );
      const t0 = performance.now();
      const resp = await fetch(cmrUrl, { signal: dispAbort.signal });
      if (!resp.ok) throw new Error(`CMR returned HTTP ${resp.status}`);
      const body = await resp.text();
      const tWasm0 = performance.now();
      lastDispResult = JSON.parse(wasmMod.parse_disp_cmr_page_json(body, urlType, dedup));
      const wasmMs = performance.now() - tWasm0;

      const summary = $("op-disp-summary");
      clearEl(summary);
      addStat(summary, lastDispResult.total_hits.toLocaleString(), "CMR total hits");
      addStat(summary, lastDispResult.products.length, "Products returned");
      addStat(summary, lastDispResult.duplicates_removed, "Duplicates removed");
      addStat(summary, `${wasmMs.toFixed(1)} ms`, "WASM parse & dedup");
      summary.hidden = false;

      const tbody = $("op-disp-tbody");
      clearEl(tbody);
      for (const p of lastDispResult.products) {
        const tr = document.createElement("tr");
        const vals = [
          `F${String(p.frame_id).padStart(5, "0")}`,
          p.polarization,
          p.reference_datetime.slice(0, 10),
          p.secondary_datetime.slice(0, 10),
          `v${p.version}`,
          p.generation_datetime.slice(0, 10),
        ];
        for (const v of vals) {
          const td = document.createElement("td");
          td.textContent = v;
          tr.appendChild(td);
        }
        const linkTd = document.createElement("td");
        if (p.filename.startsWith("https://") || p.filename.startsWith("http://")) {
          const a = document.createElement("a");
          a.href = p.filename;
          a.target = "_blank";
          a.rel = "noopener noreferrer";
          a.textContent = p.filename.split("/").pop();
          linkTd.appendChild(a);
        } else {
          linkTd.textContent = p.filename;
        }
        tr.appendChild(linkTd);
        tbody.appendChild(tr);
      }
      $("op-disp-results").hidden = false;
      setStatus(
        `Found ${lastDispResult.products.length} DISP-S1 products for frame ${frameId} in ${(performance.now() - t0).toFixed(0)} ms.`
      );
    } catch (e) {
      if (e && e.name === "AbortError") return;
      setStatus(`DISP-S1 search failed: ${errMsg(e)}`, true);
    } finally {
      btn.disabled = false;
    }
  }

  function applyExplorerPreset(key) {
    const p = EXPLORER_PRESETS[key];
    if (!p) return;
    $("op-product").value = p.product;
    $("op-band").value = p.band;
    $("op-start").value = p.start;
    $("op-end").value = p.end;
    $("op-bbox").value = p.bbox;
    $("op-water-only").checked = p.waterOnly;
    populatePresetsDropdown();
    if (p.expression) $("op-preset-expr").value = p.expression;
    updateLegend();
    updateEquivalentCommands();
    if (wasmMod) runExplorerSearch();
  }

  function teardown() {
    if (activeAbort) {
      activeAbort.abort();
      activeAbort = null;
    }
    if (dispAbort) {
      dispAbort.abort();
      dispAbort = null;
    }
    if (map) {
      map.remove();
      map = null;
      mapReady = false;
    }
  }

  function init() {
    const root = $("opera-playground");
    if (!root) {
      teardown();
      return;
    }
    if (root.dataset.initialized === "1") return;
    root.dataset.initialized = "1";
    teardown();

    // Wire tabs
    const tabs = Array.from(root.querySelectorAll(".op-tab"));
    const panels = {
      explorer: $("op-panel-explorer"),
      bursts: $("op-panel-bursts"),
      missing: $("op-panel-missing"),
      disp: $("op-panel-disp"),
    };
    tabs.forEach((tab) => {
      tab.addEventListener("click", () => {
        const target = tab.dataset.tab;
        tabs.forEach((t) => {
          const active = t.dataset.tab === target;
          t.classList.toggle("op-tab--active", active);
          t.setAttribute("aria-selected", active ? "true" : "false");
        });
        Object.entries(panels).forEach(([k, panel]) => {
          if (panel) panel.hidden = k !== target;
        });
        if (target === "explorer" && map) map.resize();
        if (target === "missing") drawMissingDataMatrix();
      });
    });

    // Wire Tab 1 controls
    $("op-preset-select").addEventListener("change", (e) => {
      if (e.target.value !== "custom") applyExplorerPreset(e.target.value);
    });
    const markCustomPreset = () => {
      const sel = $("op-preset-select");
      if (sel) sel.value = "custom";
    };
    $("op-product").addEventListener("change", () => {
      markCustomPreset();
      const prod = currentProductObj();
      if (prod) {
        $("op-band").value = prod.default_bands[0] || (prod.backend === "xarray" ? "displacement" : "");
        $("op-water-only").checked = prod.short_name.includes("DSWX");
      }
      populatePresetsDropdown();
      updateLegend();
      updateEquivalentCommands();
    });
    $("op-band").addEventListener("change", () => {
      markCustomPreset();
      populatePresetsDropdown();
      updateLegend();
      updateEquivalentCommands();
    });
    $("op-bbox").addEventListener("change", () => {
      markCustomPreset();
      updateEquivalentCommands();
    });
    $("op-start").addEventListener("change", () => {
      markCustomPreset();
      updateEquivalentCommands();
    });
    $("op-end").addEventListener("change", () => {
      markCustomPreset();
      updateEquivalentCommands();
    });
    $("op-preset-expr").addEventListener("change", () => {
      updateEquivalentCommands();
      if (lastSearch && lastSearch.concept_id) applyTileLayerFromState();
    });
    $("op-water-only").addEventListener("change", () => {
      updateLegend();
      updateEquivalentCommands();
      if (lastSearch && lastSearch.concept_id) applyTileLayerFromState();
    });
    $("op-opacity").addEventListener("input", () => {
      if (map && map.getLayer("opera-tiles-layer")) {
        map.setPaintProperty(
          "opera-tiles-layer",
          "raster-opacity",
          parseFloat($("op-opacity").value || "0.85")
        );
      }
    });
    $("op-search-btn").addEventListener("click", runExplorerSearch);
    $("op-stats-btn").addEventListener("click", computeZonalStats);
    $("op-geocode-btn").addEventListener("click", geocodePlace);
    $("op-place").addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        geocodePlace();
      }
    });
    $("op-unpin-btn").addEventListener("click", () => {
      pinnedGranuleUr = null;
      for (const row of $("op-granules-tbody").querySelectorAll("tr")) {
        row.classList.remove("op-row-selected");
      }
      applyTileLayerFromState();
    });
    $("op-dl-geojson-btn").addEventListener("click", () => {
      if (!lastSearch) return;
      downloadText(
        "opera_footprints.geojson",
        JSON.stringify(lastSearch.feature_collection, null, 2),
        "application/geo+json"
      );
    });

    // Wire Tab 2 (Bursts & DISP-S1 Parser)
    $("op-burst-input").value = SAMPLE_CSLC_LINES.join("\n");
    $("op-burst-sample-select").addEventListener("change", (e) => {
      $("op-burst-input").value =
        e.target.value === "disp" ? SAMPLE_DISP_LINES.join("\n") : SAMPLE_CSLC_LINES.join("\n");
      runBurstParser();
    });
    $("op-parse-btn").addEventListener("click", runBurstParser);

    // Wire Tab 3 (Missing-Data Optimizer)
    $("op-md-sample-select").addEventListener("change", (e) => {
      if (e.target.value === "27x24") {
        loadMissingDataPreset(27, 24, [
          [0, 2],
          [0, 7],
          [0, 15],
          [5, 4],
          [5, 15],
          [11, 7],
          [11, 19],
          [18, 11],
          [18, 22],
          [24, 2],
          [24, 11],
          [24, 22],
        ]);
      } else {
        loadMissingDataPreset(12, 10, [
          [0, 2],
          [0, 6],
          [3, 4],
          [7, 2],
          [7, 8],
          [10, 4],
          [10, 8],
        ]);
      }
    });
    $("op-md-random-gaps").addEventListener("click", () => {
      for (let k = 0; k < 3; k++) {
        const bi = Math.floor(Math.random() * mdState.burstIds.length);
        const di = Math.floor(Math.random() * mdState.dates.length);
        mdState.missing.add(`${bi}:${di}`);
      }
      runMissingDataOptimizer();
    });
    $("op-md-canvas").addEventListener("click", (e) => {
      if (!mdState.burstIds.length) return;
      const canvas = e.currentTarget;
      const rect = canvas.getBoundingClientRect();
      const x = ((e.clientX - rect.left) / rect.width) * canvas.width;
      const y = ((e.clientY - rect.top) / rect.height) * canvas.height;
      const { padL, padT, cellW, cellH } = mdMatrixGeometry(canvas);
      const di = Math.floor((x - padL) / cellW);
      const bi = Math.floor((y - padT) / cellH);
      if (bi < 0 || bi >= mdState.burstIds.length || di < 0 || di >= mdState.dates.length) return;
      const key = `${bi}:${di}`;
      if (mdState.missing.has(key)) mdState.missing.delete(key);
      else mdState.missing.add(key);
      runMissingDataOptimizer();
    });
    $("op-md-dl-txt").addEventListener("click", () => {
      const opt = mdState.options[mdState.selectedIdx];
      if (!opt) return;
      downloadText(
        `option_${mdState.selectedIdx + 1}_bursts_${opt.total_num_bursts}.txt`,
        opt.inputs.join("\n") + "\n"
      );
    });
    $("op-md-dl-json").addEventListener("click", () => {
      if (!mdState.options.length) return;
      downloadText(
        "missing_data_options.json",
        JSON.stringify(mdState.options, null, 2),
        "application/json"
      );
    });

    // Wire Tab 4 (DISP-S1 Frame Search)
    $("op-disp-preset-select").addEventListener("change", (e) => {
      if (e.target.value !== "custom") {
        $("op-disp-frame").value = e.target.value;
        runDispSearch();
      }
    });
    $("op-disp-frame").addEventListener("change", () => {
      const val = $("op-disp-frame").value;
      const sel = $("op-disp-preset-select");
      if (sel) {
        const hasOpt = Array.from(sel.options).some((o) => o.value === val);
        sel.value = hasOpt ? val : "custom";
      }
    });
    $("op-disp-search-btn").addEventListener("click", runDispSearch);
    $("op-disp-dl-urls").addEventListener("click", () => {
      if (!lastDispResult) return;
      const urls = lastDispResult.products.map((p) => p.filename).join("\n") + "\n";
      downloadText(`disp_frame_${$("op-disp-frame").value}_urls.txt`, urls);
    });
    $("op-disp-dl-json").addEventListener("click", () => {
      if (!lastDispResult) return;
      downloadText(
        `disp_frame_${$("op-disp-frame").value}.json`,
        JSON.stringify(lastDispResult, null, 2),
        "application/json"
      );
    });

    // Initialize WebAssembly module
    loadWasm()
      .then((mod) => {
        wasmMod = mod;
        catalog = JSON.parse(mod.catalog_json());
        const prodSel = $("op-product");
        clearEl(prodSel);
        for (const p of catalog) {
          const opt = document.createElement("option");
          opt.value = p.short_name;
          opt.textContent = `${p.short_title} (${p.short_name})`;
          prodSel.appendChild(opt);
        }
        $("op-search-btn").disabled = false;
        $("op-parse-btn").disabled = false;
        $("op-disp-search-btn").disabled = false;

        populatePresetsDropdown();
        updateLegend();
        updateEquivalentCommands();
        runBurstParser();
        loadMissingDataPreset(12, 10, [
          [0, 2],
          [0, 6],
          [3, 4],
          [7, 2],
          [7, 8],
          [10, 4],
          [10, 8],
        ]);
        setStatus(`opera-rs v${mod.version()} WebAssembly ready (zero GDAL, client-side). Pick a preset or click Search & Render.`);
        ensureMap().catch((e) => setStatus(`MapLibre failed to load: ${errMsg(e)}`, true));
      })
      .catch((e) => {
        setStatus(`Failed to load opera-rs WebAssembly module: ${errMsg(e)}`, true);
      });
  }

  if (window.document$) {
    window.document$.subscribe(init);
  } else {
    document.addEventListener("DOMContentLoaded", init);
  }
})();
