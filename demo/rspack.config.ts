import type { RspackOptions } from "@rspack/core";
import { ReactPlugin } from "@rspack/plugin-react";

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
        loader: "builtin:css-loader",
      },
    ],
  },
  plugins: [new ReactPlugin()],
  builtins: {
    html: [
      {
        template: "./src/index.html",
      },
    ],
    copy: [
      {
        from: "public",
        to: ".",
      },
    ],
  },
  devServer: {
    static: {
      directory: "public",
    },
    port: 8080,
    hot: true,
  },
};

export default config;
