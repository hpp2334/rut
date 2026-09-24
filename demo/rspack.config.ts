import type { RspackOptions } from "@rspack/core";
import { CopyRspackPlugin, HtmlRspackPlugin, rspack } from "@rspack/core";
import { ReactRefreshRspackPlugin } from "@rspack/plugin-react-refresh";

// react-refresh is a DEV-SERVER feature: its runtime ($RefreshReg$/
// $RefreshSig$) comes from `rspack serve`. A static `npm run build`
// bundle emitted the registration CALLS with no runtime — the shipped
// page died on load with "ReferenceError: $RefreshReg$ is not defined"
// and #root stayed empty (found by the phase-4 browser drive; the
// smoke's node bundle never mounts React, so it couldn't see it).
// npm sets npm_lifecycle_event per script, so the refresh machinery
// rides the `dev` lane only.
//
// The same split ships PRODUCTION React in dist (the journey audit's
// L-1 ride-along): the build lane is mode "production", so the
// development-only validators the M-1 profile caught burning ~27% of
// active CPU (warnInvalidARIAProps, the key-prop warnings, jsxDEV) are
// compiled out of the shipped bundle. The dev lane keeps mode
// "development" for refresh + HMR + the dev-only double-render checks.
const isDevServe = process.env.npm_lifecycle_event === "dev";

const config: RspackOptions = {
  context: __dirname,
  entry: "./src/main.tsx",
  mode: isDevServe ? "development" : "production",
  devtool: isDevServe ? "source-map" : false,
  output: {
    filename: "[name].js",
    path: "./dist",
    clean: true,
  },
  resolve: {
    extensions: [".tsx", ".ts", ".js"],
  },
  module: {
    rules: [
      {
        test: /\.tsx?$/,
        exclude: /node_modules/,
        loader: "builtin:swc-loader",
        options: {
          jsc: {
            parser: { syntax: "typescript", tsx: true },
            transform: {
              react: {
                runtime: "automatic",
                // jsx vs jsxDEV rides the lane: the shipped bundle gets
                // the production jsx runtime (the dev validators die
                // with it), the dev server keeps the dev runtime
                development: isDevServe,
                refresh: isDevServe,
              },
            },
          },
        },
      },
      {
        test: /\.css$/,
        type: "css",
      },
      // the classics (RFC 0041 §3): example sources imported as strings
      // by demo/src/examples/index.ts — rut-ONLY since the no-sidecars
      // batch (expected rides inline in the case entries), so a leftover
      // `.expected` import fails the build loudly
      {
        test: /\.rut$/,
        type: "asset/source",
      },
    ],
  },
  plugins: [
    ...(isDevServe ? [new ReactRefreshRspackPlugin()] : []),
    new HtmlRspackPlugin({ template: "./src/index.html" }),
    // ship rut.wasm with the bundle (RFC 0041 §3: the runner probes it)
    new CopyRspackPlugin({
      patterns: [{ from: "public", to: "." }],
    }),
  ],
  devServer: {
    static: {
      directory: "public",
    },
    port: 8080,
    hot: true,
  },
};

export default config;
