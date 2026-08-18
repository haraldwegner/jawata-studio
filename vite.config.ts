import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // `npm run tauri dev` died here with ENOSPC — the dev server watched
      // src-tauri/target, which holds ~143k incremental build artifacts, and
      // exhausted the OS inotify limit. Vite then exited, the Tauri window had
      // no frontend to load, and the app came up WHITE: a frontend failure
      // wearing a Rust failure's clothes.
      //
      // Rust sources are watched by the Tauri CLI itself ("Watching
      // .../src-tauri for changes"), so this side never needed them.
      ignored: ["**/src-tauri/**"]
    }
  }
});
