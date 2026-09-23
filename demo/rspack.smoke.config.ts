import type { RspackOptions } from "@rspack/core";

/**
 * The smoke bundle (demo-owned): the app's own RutApi surface bundled
 * for node, so the headless gate drives the SAME code the browser runs
 * (src/runner.ts + src/cases.ts + src/verify.ts + src/examples) — the
 * lsp-align "drive the shipped artifact through the shipped binding"
 * pattern. No React, no DOM.
 */
const config: RspackOptions = {
  context: __dirname,
  entry: "./src/smoke/smoke-entry.ts",
  target: "node",
  mode: "development",
  devtool: false,
  output: {
    filename: "rut-api.cjs",
    path: "./dist-smoke",
    library: { type: "commonjs2" },
  },
  resolve: {
    extensions: [".tsx", ".ts", ".js"],
  },
  module: {
    rules: [
      {
        test: /\.ts$/,
        exclude: /node_modules/,
        loader: "builtin:swc-loader",
        options: {
          jsc: {
            parser: { syntax: "typescript", tsx: false },
          },
        },
      },
      // the classics: example sources as strings — rut-ONLY since the
      // no-sidecars batch (expected rides inline in the case entries),
      // so a leftover `.expected` import fails the build loudly
      {
        test: /\.rut$/,
        type: "asset/source",
      },
    ],
  },
};

export default config;
