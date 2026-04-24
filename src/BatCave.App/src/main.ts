import "uplot/dist/uPlot.min.css";
import "./app.css";
import { mount } from "svelte";
import App from "./App.svelte";

const target = document.getElementById("app");

if (!target) {
  throw new Error("BatCave monitor root element was not found.");
}

export default mount(App, { target });
