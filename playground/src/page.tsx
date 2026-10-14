import React from "react";
import { createRoot } from "react-dom/client";
import handbook from "./generated/handbook.json";
// The upstream page is transformed only at its compiler bootstrap boundary.
import Play from "@tsrs/page";

const globals = window as typeof window & Record<string, any>;
globals.global = window;
const theme = localStorage.getItem("force-color-theme");
if ((!theme && matchMedia("(prefers-color-scheme: dark)").matches) || theme === "force-dark")
  document.documentElement.classList.add("dark-theme");
else if (theme === "force-light") document.documentElement.classList.add("light-theme");
document.documentElement.classList.add(
  "font-" + (localStorage.getItem("force-font") ?? "cascadia"),
);

const response = await fetch(new URL("js/examples/en.json", document.baseURI));
if (!response.ok) throw new Error(`Examples could not be loaded (${response.status}).`);
const examplesTOC = await response.json();
createRoot(document.getElementById("gatsby-focus-wrapper")!).render(
  <Play
    pageContext={{
      lang: "en",
      examplesTOC,
      optionsSummary: [],
      playgroundHandbookTOC: { docs: handbook.docs },
    }}
  />,
);
