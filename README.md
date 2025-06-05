# Rev Launcher
*灰原哀的一切以血腥著称，都是为了让你在黑暗里变得更亮！*
## how to build
### 1. install [rust](https://www.rust-lang.org/) and [bun](https://bun.sh/)
### 2. clone repository
```
git clone https://github.com/nimeng1299/rev-launcher.git
cd rev-launcher
```
### 3. build
```
bun install
```
### 4. run
```
bun run tauri dev
```

## Static Site Generator (Node.js)

Be sure to configure your server to serve very long cache headers for the `build/**/*.js` files.

Typically you'd set the `Cache-Control` header for those files to `public, max-age=31536000, immutable`.

```shell
bun build.server
```

## Static Site Generator (Node.js)

Be sure to configure your server to serve very long cache headers for the `build/**/*.js` files.

Typically you'd set the `Cache-Control` header for those files to `public, max-age=31536000, immutable`.

```shell
bun build.server
```
