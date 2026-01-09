import { transformAsync } from "@babel/core";
// @ts-expect-error - preset types are not exposed
import ts from "@babel/preset-typescript";
// @ts-expect-error - preset types are not exposed
import solid from "babel-preset-solid";
import { plugin, type BunPlugin } from "bun";

const pragmaPattern = /@jsxImportSource\s+@opentui\/solid/;

const solidTestTransformPlugin: BunPlugin = {
  name: "bun-plugin-solid-test",
  setup: (build) => {
    build.onLoad(
      { filter: /\/node_modules\/solid-js\/dist\/server\.js$/ },
      async (args) => {
        const path = args.path.replace("server.js", "solid.js");
        const file = Bun.file(path);
        const code = await file.text();
        return { contents: code, loader: "js" };
      },
    );
    build.onLoad(
      { filter: /\/node_modules\/solid-js\/store\/dist\/server\.js$/ },
      async (args) => {
        const path = args.path.replace("server.js", "store.js");
        const file = Bun.file(path);
        const code = await file.text();
        return { contents: code, loader: "js" };
      },
    );
    build.onLoad({ filter: /\.(js|ts)x$/ }, async (args) => {
      const file = Bun.file(args.path);
      const code = await file.text();
      if (!pragmaPattern.test(code)) {
        return null;
      }
      const transforms = await transformAsync(code, {
        filename: args.path,
        presets: [
          [
            solid,
            {
              moduleName: "@opentui/solid",
              generate: "universal",
            },
          ],
          [ts],
        ],
      });
      return {
        contents: transforms?.code ?? "",
        loader: "js",
      };
    });
  },
};

plugin(solidTestTransformPlugin);
