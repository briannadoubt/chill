# Vendored Chill browser SDK

The console consumes the repository's real `@chill-observability/browser` package. Its release tarball is committed here because the Sites source repository contains the console subtree and must remain independently installable.

Run `npm run vendor:sdk` from `console/` after changing the browser SDK, then reinstall dependencies to refresh `package-lock.json`.
