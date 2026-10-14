import React from "react";

/** Preserve official site links while routing playground/assets under any static base path. */
export function withPrefix(value: string) {
  const normalized = value.startsWith("/") ? value : "/" + value;
  if (normalized.startsWith("/play"))
    return new URL("." + normalized.slice(5), document.baseURI).href;
  if (/^\/(?:js|css|fonts)\//.test(normalized))
    return new URL(normalized.slice(1), document.baseURI).href;
  if (normalized === "/") return new URL("./", document.baseURI).href;
  return "https://www.typescriptlang.org" + normalized;
}

export function Link({
  to,
  children,
  ...props
}: React.AnchorHTMLAttributes<HTMLAnchorElement> & { to: string }) {
  return (
    <a {...props} href={withPrefix(to)}>
      {children}
    </a>
  );
}
