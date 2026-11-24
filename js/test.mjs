import assert from "node:assert/strict";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const opera = require("./pkg-node/opera_js.js");

// 1. Version & catalog
assert.ok(opera.version().length > 0);
const catalog = JSON.parse(opera.catalog_json());
assert.equal(catalog.length, 9);

const dswx = JSON.parse(opera.find_product_json("dswx-hls"));
assert.equal(dswx.short_name, "OPERA_L3_DSWX-HLS_V1");

const presets = JSON.parse(opera.expression_presets_json("rtc-s1", "VV"));
assert.ok(presets.length >= 1);

const legend = JSON.parse(opera.legend_entries_json("dswx-hls", "B01_WTR", true));
assert.ok(legend.some((e) => e.label === "Open water"));

// 2. CMR URL builder & titiler-cmr URLs
const cmrUrl = opera.build_cmr_search_url_js(
  "dswx-hls",
  -119.95,
  35.85,
  -119.45,
  36.25,
  "2023-04-20",
  "2023-04-30",
  25
);
assert.ok(cmrUrl.includes("short_name=OPERA_L3_DSWX-HLS_V1"));
assert.ok(cmrUrl.includes("bounding_box="));

const urls = JSON.parse(
  opera.build_titiler_urls_json(
    "C2617126679-POCLOUD",
    "dswx-hls",
    "rasterio",
    null,
    "2023-04-20/2023-04-30",
    "B01_WTR",
    true,
    null,
    null,
    null,
    256
  )
);
assert.equal(urls.categorical, true);
assert.ok(urls.tile_url_template.includes("tiles/WebMercatorQuad/{z}/{x}/{y}.png"));
assert.ok(urls.statistics_url.includes("/statistics"));

const ptUrl = opera.build_point_url_js(
  "C2617126679-POCLOUD",
  "dswx-hls",
  "rasterio",
  null,
  "2023-04-20/2023-04-30",
  "B01_WTR",
  null,
  -119.72,
  36.05,
  null
);
assert.ok(ptUrl.includes("/point/-119.72,36.05"));

// 3. Sentinel-1 bursts & DISP-S1 filenames
assert.equal(opera.normalize_burst_id_js("T087-185683-IW2"), "t087_185683_iw2");

const cslc =
  "OPERA_L2_CSLC-S1_T087-185683-IW2_20230322T161649Z_20240504T185235Z_S1A_VV_v1.1.h5";
const parsedCslc = JSON.parse(opera.parse_cslc_filename_json(cslc));
assert.equal(parsedCslc.burst_id, "t087_185683_iw2");
assert.deepEqual(JSON.parse(opera.get_dates_json(cslc)), ["2023-03-22", "2024-05-04"]);

const disp1 =
  "OPERA_L3_DISP-S1_IW_F11114_VV_20160705T140755Z_20230418T140812Z_v1.0_20241219T231545Z.nc";
const disp2 =
  "OPERA_L3_DISP-S1_IW_F11114_VV_20160705T140755Z_20230418T140812Z_v1.0_20250101T000000Z.nc";
const parsedDisp = JSON.parse(opera.parse_disp_filename_json(disp1));
assert.equal(parsedDisp.frame_id, 11114);
const deduped = JSON.parse(opera.deduplicate_disp_filenames_json(JSON.stringify([disp1, disp2])));
assert.equal(deduped.products.length, 1);
assert.equal(deduped.duplicates_removed, 1);

// 4. DISP-S1 CMR URL builder & wrapped UMM-G parser
const dispUrl = opera.build_disp_search_url_js(
  11114,
  null,
  null,
  null,
  null,
  null,
  null,
  "1.0",
  25
);
assert.ok(dispUrl.includes("FRAME_NUMBER%2C11114"));
assert.ok(dispUrl.includes("float%2CPRODUCT_VERSION%2C1.0"));

const dispUmmPage = JSON.stringify({
  hits: 2,
  items: [
    {
      umm: {
        GranuleUR: disp1.replace(/\.nc$/, ""),
        RelatedUrls: [
          {
            Type: "GET DATA",
            URL: `https://datapool.asf.alaska.edu/DISP/OPERA-S1/${disp1}`,
          },
        ],
      },
    },
    {
      umm: {
        GranuleUR: disp2.replace(/\.nc$/, ""),
        RelatedUrls: [
          {
            Type: "GET DATA",
            URL: `https://datapool.asf.alaska.edu/DISP/OPERA-S1/${disp2}`,
          },
        ],
      },
    },
  ],
});
const dispParsedPage = JSON.parse(opera.parse_disp_cmr_page_json(dispUmmPage, "https", true));
assert.equal(dispParsedPage.total_hits, 2);
assert.equal(dispParsedPage.products.length, 1);
assert.equal(dispParsedPage.duplicates_removed, 1);
assert.equal(dispParsedPage.products[0].frame_id, 11114);

// 5. Point and categorical statistics parsers (verify exact JSON field names used by Playground)
const pointFixture = JSON.stringify({
  coordinates: [-119.72, 36.05],
  assets: [{ name: "B01_WTR", values: [1.0], band_names: ["b1"] }],
});
const parsedPoint = JSON.parse(opera.parse_point_response_json(pointFixture, "dswx-hls", "B01_WTR"));
assert.equal(parsedPoint.assets[0].class_labels[0], "Open water");

const statsFixture = JSON.stringify({
  properties: {
    statistics: {
      B01_WTR_b1: {
        min: 0,
        max: 1,
        mean: 0.25,
        count: 1000,
        valid_pixels: 1000,
        valid_percent: 100,
        histogram: [
          [750, 250],
          [0, 1],
        ],
      },
    },
  },
});
const parsedStats = JSON.parse(
  opera.parse_statistics_response_json(statsFixture, "dswx-hls", "B01_WTR", true)
);
const cls0 = parsedStats.bands.B01_WTR_b1.class_breakdown[0];
assert.equal(typeof cls0.class_value, "number");
assert.equal(typeof cls0.pixel_count, "number");
assert.equal(typeof cls0.percent, "number");
assert.equal(typeof cls0.area_km2, "number");
assert.ok(parsedStats.bands.B01_WTR_b1.class_breakdown.some((c) => c.label === "Open water"));

// Unknown products must fail consistently
assert.throws(() => opera.parse_point_response_json(pointFixture, "not-a-product", "B01_WTR"));
assert.throws(() => opera.expression_presets_json("not-a-product", "B01_WTR"));

// 6. Missing-data subset optimization
const stack = [
  "t087_185683_iw1_20230101.h5",
  "t087_185683_iw1_20230113.h5",
  "t087_185683_iw2_20230101.h5",
];
const opts = JSON.parse(opera.get_missing_data_options_json(JSON.stringify(stack), 10));
assert.ok(opts.length >= 2);
assert.equal(opts[0].total_num_bursts, 2);

console.log("opera-js WASM smoke tests passed!");
