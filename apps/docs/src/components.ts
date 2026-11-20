/**
 * MDX globals registry — components available inside MDX without `import`.
 * Wired via `<Content components={components} />` in `[...slug].astro`.
 * Add new components here as you build (or install) them.
 */

import { Aside } from "./components/ui/aside";
import Render from "./components/Render.astro";
import { Card } from "./components/ui/card";
import { CardGrid } from "./components/ui/card-grid";
import { PackageManagers } from "./components/ui/package-managers";
import { Step, Steps } from "./components/ui/steps";
import { Tabs, TabItem } from "./components/ui/tabs";
import SsrDeploySequence from "./components/diagrams/SsrDeploySequence.astro";
import SsrRequestFlow from "./components/diagrams/SsrRequestFlow.astro";
import SsrRequestSequence from "./components/diagrams/SsrRequestSequence.astro";
import SsrTopology from "./components/diagrams/SsrTopology.astro";

export const components = {
  Aside,
  Card,
  CardGrid,
  PackageManagers,
  Render,
  Step,
  Steps,
  SsrDeploySequence,
  SsrRequestFlow,
  SsrRequestSequence,
  SsrTopology,
  TabItem,
  Tabs,
};
