/* The entry: mount the app and pull in the Tailwind stylesheet. */
import { createRoot } from "react-dom/client";
import { App } from "./app.jsx";
import "./styles.css";

createRoot(document.getElementById("root")).render(<App />);
