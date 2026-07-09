import "uplot/dist/uPlot.min.css";
import "./styles/tokens.css";
import "./styles/themes.css";
import "./styles/base.css";
import "./styles/components.css";
import "./styles/redesign.css";
import { mount } from "svelte";
import App from "./App.svelte";

const target = document.getElementById("app");

if (!target) {
  throw new Error("BatCave monitor root element was not found.");
}

export default mount(App, { target });
