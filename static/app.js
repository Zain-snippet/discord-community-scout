// Frontend entry point. Plain ES module, no build step.

const statusLine = document.getElementById("status-line");
const statusFacts = document.getElementById("status-facts");

/**
 * Render the server's health response. Uses textContent only, never innerHTML,
 * so nothing the server returns can inject markup.
 */
async function loadHealth() {
  try {
    const response = await fetch("/api/health", { headers: { Accept: "application/json" } });
    const body = await response.json();
    const ok = response.ok && body.status === "ok";

    statusLine.replaceChildren();
    const pill = document.createElement("span");
    pill.className = `pill ${ok ? "pill-ok" : "pill-danger"}`;
    pill.textContent = ok ? "Operational" : "Degraded";
    statusLine.append(pill);

    statusFacts.replaceChildren();
    for (const [label, value] of [
      ["Version", body.version ?? "unknown"],
      ["Database", body.database ?? "unknown"],
    ]) {
      const dt = document.createElement("dt");
      dt.textContent = label;
      const dd = document.createElement("dd");
      dd.textContent = String(value);
      statusFacts.append(dt, dd);
    }
  } catch (error) {
    statusLine.replaceChildren();
    const pill = document.createElement("span");
    pill.className = "pill pill-danger";
    pill.textContent = "Unreachable";
    statusLine.append(pill, " The server did not respond.");
    console.error("health check failed", error);
  }
}

loadHealth();
