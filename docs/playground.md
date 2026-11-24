# Playground

Explore **`opera-rs`** directly in your browser via **WebAssembly** (`opera-js`, compiled from `opera-rs --no-default-features` with zero GDAL or C/C++ dependencies), paired with live public NASA CMR and `titiler-cmr` endpoints (**no Earthdata login required**).

<div id="opera-playground" markdown="0">
  <p class="op-status" id="op-status">Loading opera-rs WebAssembly module...</p>

  <div class="op-tabs" role="tablist" aria-label="Playground workspaces">
    <button type="button" class="op-tab op-tab--active" role="tab" aria-selected="true" data-tab="explorer">Live OPERA Map &amp; Stats</button>
    <button type="button" class="op-tab" role="tab" aria-selected="false" data-tab="bursts">Bursts &amp; DISP-S1 Parser (WASM)</button>
    <button type="button" class="op-tab" role="tab" aria-selected="false" data-tab="missing">Missing-Data Optimizer (WASM)</button>
    <button type="button" class="op-tab" role="tab" aria-selected="false" data-tab="disp">Live DISP-S1 Frame Search</button>
  </div>

  <!-- TAB 1: LIVE OPERA MAP & ANALYTICS -->
  <section class="op-panel" id="op-panel-explorer" role="tabpanel">
    <div class="op-controls">
      <div class="op-field">
        <label for="op-preset-select">Preset</label>
        <select id="op-preset-select">
          <option value="tulare" selected>Tulare Lake 2023 Flood (DSWx-HLS)</option>
          <option value="valencia">Valencia 2024 Flood (DSWx-S1)</option>
          <option value="gsl">Great Salt Lake (DSWx-HLS)</option>
          <option value="la-fire">Los Angeles 2025 Fire (DIST-ALERT)</option>
          <option value="houston">Houston SAR Backscatter (RTC-S1)</option>
          <option value="custom">Custom</option>
        </select>
      </div>
      <div class="op-field">
        <label for="op-product">OPERA Product</label>
        <select id="op-product"></select>
      </div>
      <div class="op-field">
        <label for="op-band">Band / Variable</label>
        <input type="text" id="op-band" value="B01_WTR" placeholder="B01_WTR">
      </div>
      <div class="op-field">
        <label for="op-preset-expr">Expression Preset</label>
        <select id="op-preset-expr">
          <option value="">Default band styling</option>
        </select>
      </div>
      <div class="op-field">
        <label for="op-start">Start Date</label>
        <input type="date" id="op-start" value="2023-04-20">
      </div>
      <div class="op-field">
        <label for="op-end">End Date</label>
        <input type="date" id="op-end" value="2023-04-30">
      </div>
      <div class="op-field op-field--wide">
        <label for="op-bbox">WGS 84 BBox (W, S, E, N)</label>
        <input type="text" id="op-bbox" value="-119.95,35.85,-119.45,36.25">
      </div>
      <div class="op-field">
        <label for="op-place">Geocode Place (Nominatim)</label>
        <div class="op-inline-input">
          <input type="text" id="op-place" placeholder="e.g. Tulare Lake, CA">
          <button type="button" id="op-geocode-btn" class="op-icon-btn" title="Resolve place name to bbox">Go</button>
        </div>
      </div>
      <div class="op-field op-field--check">
        <label class="op-inline-check"><input type="checkbox" id="op-water-only" checked> Water-only mask</label>
        <label class="op-inline-check"><input type="checkbox" id="op-dedup" checked> Deduplicate versions</label>
      </div>
      <div class="op-field op-action">
        <button type="button" id="op-search-btn" class="md-button md-button--primary" disabled>Search &amp; Render</button>
      </div>
    </div>

    <div class="op-summary" id="op-explorer-summary" hidden></div>

    <div class="op-explorer-stack">
      <div class="op-map-card">
        <div class="op-map-toolbar">
          <span class="op-hint" id="op-map-hint">Click Search &amp; Render to load live NASA CMR footprints and titiler-cmr tiles. Click anywhere on the map to sample pixel values.</span>
          <label class="op-inline-check">
            Opacity
            <input type="range" id="op-opacity" min="0" max="1" step="0.05" value="0.85">
          </label>
        </div>
        <div id="op-map" class="op-map"></div>
        <div class="op-legend" id="op-legend"></div>
        <div class="op-point-box" id="op-point-box" hidden></div>
      </div>

      <div class="op-side-card">
        <div class="op-card-head">
          <h4>Zonal AOI Statistics (30 m)</h4>
          <button type="button" id="op-stats-btn" class="op-icon-btn" disabled>Compute Stats</button>
        </div>
        <p class="op-hint" id="op-stats-note">Computes live zonal statistics from <code>titiler-cmr</code> and calculates 30 m class areas (km<sup>2</sup>) in Rust WebAssembly.</p>
        <div class="op-stats-grid">
          <div>
            <canvas id="op-stats-chart" width="520" height="220"></canvas>
          </div>
          <div>
            <div id="op-stats-table-wrap" class="op-mini-table-wrap" hidden>
              <table class="op-table">
                <thead>
                  <tr><th>Class / Metric</th><th>Pixels</th><th>%</th><th>Area (km<sup>2</sup>)</th></tr>
                </thead>
                <tbody id="op-stats-tbody"></tbody>
              </table>
            </div>
            <details class="op-code-details" open>
              <summary>Equivalent CLI &amp; Python Commands</summary>
              <pre><code id="op-equiv-cmd"></code></pre>
            </details>
          </div>
        </div>
      </div>
    </div>

    <div id="op-granules-section" hidden>
      <div class="op-card-head">
        <h4>CMR Granules <span class="op-hint" id="op-granules-note">(click a row to pin map tiles to that granule)</span></h4>
        <div class="op-btn-row">
          <button type="button" id="op-unpin-btn" class="op-icon-btn" hidden>Show Date Composite</button>
          <button type="button" id="op-dl-geojson-btn" class="op-icon-btn">Download Footprints GeoJSON</button>
        </div>
      </div>
      <div class="op-table-wrap">
        <table class="op-table">
          <thead>
            <tr>
              <th>Date</th>
              <th>Time</th>
              <th>Tile / Burst</th>
              <th>Bands</th>
              <th>Granule UR</th>
            </tr>
          </thead>
          <tbody id="op-granules-tbody"></tbody>
        </table>
      </div>
    </div>
  </section>

  <!-- TAB 2: BURST & DISP-S1 PARSER (WASM) -->
  <section class="op-panel" id="op-panel-bursts" role="tabpanel" hidden>
    <div class="op-inline-bar">
      <label for="op-burst-sample-select">Sample dataset</label>
      <select id="op-burst-sample-select">
        <option value="cslc" selected>CSLC-S1 + COMPRESSED-CSLC-S1 Filenames</option>
        <option value="disp">DISP-S1 Stack with Superseded Versions</option>
      </select>
    </div>

    <div class="op-field">
      <label for="op-burst-input">Sentinel-1 CSLC or DISP-S1 filenames / URLs (one per line, parsed 100% locally in WebAssembly)</label>
      <textarea id="op-burst-input" rows="7" spellcheck="false"></textarea>
    </div>
    <div class="op-btn-row op-mt">
      <button type="button" id="op-parse-btn" class="md-button md-button--primary" disabled>Parse &amp; Group in WASM</button>
      <span class="op-hint" id="op-parse-timing"></span>
    </div>

    <div class="op-summary" id="op-burst-summary" hidden></div>
    <div id="op-burst-results" hidden>
      <h4>Parsed Items</h4>
      <div class="op-table-wrap">
        <table class="op-table">
          <thead>
            <tr>
              <th>Kind</th>
              <th>Burst ID / Frame</th>
              <th>Dates / Pair</th>
              <th>Pol / Ver</th>
              <th>Details</th>
            </tr>
          </thead>
          <tbody id="op-burst-tbody"></tbody>
        </table>
      </div>

      <div id="op-burst-groups-wrap" hidden>
        <h4>Grouped by Normalized Burst ID</h4>
        <div class="op-table-wrap">
          <table class="op-table">
            <thead>
              <tr><th>Burst ID</th><th>File Count</th><th>Files</th></tr>
            </thead>
            <tbody id="op-burst-groups-tbody"></tbody>
          </table>
        </div>
      </div>
    </div>
  </section>

  <!-- TAB 3: MISSING-DATA SUBSET OPTIMIZER (WASM) -->
  <section class="op-panel" id="op-panel-missing" role="tabpanel" hidden>
    <div class="op-inline-bar">
      <label for="op-md-sample-select">Sample stack</label>
      <select id="op-md-sample-select">
        <option value="12x10" selected>12 Bursts x 10 Dates (7 gaps)</option>
        <option value="27x24">27 Bursts x 24 Dates (12 gaps)</option>
      </select>
      <button type="button" id="op-md-random-gaps" class="op-icon-btn">Add Random Gaps</button>
      <span class="op-hint">Tip: click any cell in the matrix below to toggle a missing acquisition live.</span>
    </div>

    <div class="op-summary" id="op-md-summary"></div>

    <div class="op-map-grid">
      <div class="op-map-card">
        <div class="op-card-head">
          <h4>Interactive Burst x Date Availability Matrix</h4>
          <span class="op-hint" id="op-md-timing"></span>
        </div>
        <canvas id="op-md-canvas" width="640" height="340" title="Click any cell to toggle missing vs available"></canvas>
        <div class="op-legend">
          <span class="op-legend-item"><i style="background:#2a9d8f"></i> Selected (active option)</span>
          <span class="op-legend-item"><i style="background:#9ca3af"></i> Available (excluded)</span>
          <span class="op-legend-item"><i style="background:#dc2626"></i> Missing acquisition</span>
        </div>
      </div>

      <div class="op-side-card">
        <div class="op-card-head">
          <h4>Ranked Optimal Subsets</h4>
          <div class="op-btn-row">
            <button type="button" id="op-md-dl-txt" class="op-icon-btn">Download .txt</button>
            <button type="button" id="op-md-dl-json" class="op-icon-btn">Download .json</button>
          </div>
        </div>
        <p class="op-hint">Click any option row to highlight its rectangular subset on the availability matrix.</p>
        <div class="op-table-wrap op-table-wrap--tall">
          <table class="op-table">
            <thead>
              <tr>
                <th>#</th>
                <th>Dates</th>
                <th>Bursts</th>
                <th>Retained</th>
                <th>Excluded</th>
              </tr>
            </thead>
            <tbody id="op-md-tbody"></tbody>
          </table>
        </div>
      </div>
    </div>
  </section>

  <!-- TAB 4: LIVE DISP-S1 FRAME SEARCH -->
  <section class="op-panel" id="op-panel-disp" role="tabpanel" hidden>
    <div class="op-controls">
      <div class="op-field">
        <label for="op-disp-preset-select">Frame Preset</label>
        <select id="op-disp-preset-select">
          <option value="11114" selected>Frame 11114 (Central California)</option>
          <option value="36541">Frame 36541 (Tulare Lake, Ascending)</option>
          <option value="20693">Frame 20693 (Houston, Texas)</option>
          <option value="custom">Custom</option>
        </select>
      </div>
      <div class="op-field">
        <label for="op-disp-frame">DISP-S1 Frame ID</label>
        <input type="number" id="op-disp-frame" value="11114" min="1" max="50000">
      </div>
      <div class="op-field">
        <label for="op-disp-ver">Product Version</label>
        <input type="text" id="op-disp-ver" value="1.0" placeholder="1.0">
      </div>
      <div class="op-field">
        <label for="op-disp-url-type">URL Type</label>
        <select id="op-disp-url-type">
          <option value="https" selected>HTTPS (ASF Datapool)</option>
          <option value="s3">S3 (Earthdata Cloud)</option>
        </select>
      </div>
      <div class="op-field">
        <label for="op-disp-limit">Limit</label>
        <input type="number" id="op-disp-limit" value="50" min="1" max="200">
      </div>
      <div class="op-field op-field--check">
        <label class="op-inline-check"><input type="checkbox" id="op-disp-dedup" checked> Deduplicate reprocessed versions</label>
      </div>
      <div class="op-field op-action">
        <button type="button" id="op-disp-search-btn" class="md-button md-button--primary" disabled>Search DISP-S1</button>
      </div>
    </div>

    <div class="op-summary" id="op-disp-summary" hidden></div>

    <div id="op-disp-results" hidden>
      <div class="op-card-head">
        <h4>DISP-S1 Products</h4>
        <div class="op-btn-row">
          <button type="button" id="op-disp-dl-urls" class="op-icon-btn">Download URL List (.txt)</button>
          <button type="button" id="op-disp-dl-json" class="op-icon-btn">Download JSON</button>
        </div>
      </div>
      <div class="op-table-wrap">
        <table class="op-table">
          <thead>
            <tr>
              <th>Frame</th>
              <th>Pol</th>
              <th>Reference Date</th>
              <th>Secondary Date</th>
              <th>Version</th>
              <th>Generated</th>
              <th>Link</th>
            </tr>
          </thead>
          <tbody id="op-disp-tbody"></tbody>
        </table>
      </div>
    </div>
  </section>
</div>
