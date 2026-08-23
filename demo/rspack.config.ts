import type { RspackOptions } from "@rspack/core";
import { CopyRspackPlugin, HtmlRspackPlugin, rspack } from "@rspack/core";
import { ReactRefreshRspackPlugin } from "@rspack/plugin-react-refresh";

const config: RspackOptions = {
  context: __dirname,
  entry: "./src/main.tsx",
  mode: "development",
  devtool: "source-map",
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
              react: { runtime: "automatic", refresh: true },
            },
          },
        },
      },
      {
        test: /\.css$/,
        type: "css",
      },
    ],
  },
  plugins: [
    new ReactRefreshRspackPlugin(),
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
