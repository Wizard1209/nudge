// A file imported with Vite's `?raw` suffix arrives as its text, inlined
// into the bundle at build time.
declare module "*.svg?raw" {
  const text: string;
  export default text;
}
