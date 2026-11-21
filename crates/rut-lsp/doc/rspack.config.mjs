import rspackPkg from "@rspack/core";
const { HtmlRspackPlugin } = rspackPkg;

export default {
  context: import.meta.dirname,
  entry: "./src/main.jsx",
  mode: "production",
  devtool: false,
  experiments: { css: true },
  output: {
    path: new URL("./dist", import.meta.url).pathname,
    clean: true,
    filename: "assets/[name].[contenthash:8].js",
  },
  module: {
    rules: [
      {
        test: /\.jsx?$/,
        exclude: /node_modules/,
        use: {
          loader: "builtin:swc-loader",
          options: {
            jsc: {
              parser: { syntax: "ecmascript", jsx: true },
              transform: { react: { runtime: "automatic" } },
            },
          },
        },
      },
      { test: /\.css$/, type: "css", use: ["postcss-loader"] },
    ],
  },
  plugins: [
    new HtmlRspackPlugin({
      template: "./src/index.html",
      meta: { viewport: "width=device-width, initial-scale=1" },
    }),
  ],
};
