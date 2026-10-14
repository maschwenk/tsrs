declare module "@tsrs/page" {
  import type { ComponentType } from "react";
  const Play: ComponentType<{
    pageContext: {
      lang: string;
      examplesTOC: unknown;
      optionsSummary: unknown[];
      playgroundHandbookTOC: { docs: unknown[] };
    };
  }>;
  export default Play;
}
