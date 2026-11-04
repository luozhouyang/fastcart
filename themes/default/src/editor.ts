import { ContactForm } from "./components/contact-form";

/**
 * Component registry consumed by the theme editor surface.
 *
 * The admin editor reads structural metadata from `editor.manifest.json`; this
 * entry is the render-side counterpart for hosts that load theme components.
 */
export const components = {
  ContactForm,
};

export default components;
