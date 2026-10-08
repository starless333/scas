(() => {
  const toggle = document.querySelector("[data-nav-toggle]");
  const rail = document.querySelector(".rail");
  if (toggle && rail) {
    const mobile = matchMedia("(max-width: 780px)");
    const setOpen = (open, restoreFocus = false) => {
      document.body.classList.toggle("nav-open", open);
      toggle.setAttribute("aria-expanded", String(open));
      toggle.setAttribute("aria-label", `${open ? "Close" : "Open"} documentation navigation`);
      rail.inert = mobile.matches && !open;
      if (restoreFocus) toggle.focus();
    };

    setOpen(false);
    toggle.addEventListener("click", () => {
      setOpen(!document.body.classList.contains("nav-open"));
    });
    rail.addEventListener("click", event => {
      if (event.target.closest("a") && mobile.matches) {
        setOpen(false);
      }
    });
    document.addEventListener("click", event => {
      if (mobile.matches && document.body.classList.contains("nav-open") &&
          !rail.contains(event.target) && !toggle.contains(event.target)) {
        setOpen(false, true);
      }
    });
    document.addEventListener("keydown", event => {
      if (event.key === "Escape" && document.body.classList.contains("nav-open")) {
        setOpen(false, true);
      }
    });
    mobile.addEventListener("change", () => setOpen(false));
  }

  document.querySelectorAll("code[data-source-file]").forEach(async code => {
    const sourcePath = code.dataset.sourceFile;
    const meta = document.querySelector("[data-source-meta]");
    code.setAttribute("aria-busy", "true");
    try {
      const response = await fetch(sourcePath);
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const source = await response.text();
      const sourceLines = source.replace(/\n$/, "").split("\n");
      const fragment = document.createDocumentFragment();
      sourceLines.forEach((line, index) => {
        const comment = line.trimStart().startsWith("///");
        const keyword = !comment && line.match(/^(\s*)(let)(\b.*)$/);
        if (comment) {
          const span = document.createElement("span");
          span.className = "source-comment";
          span.textContent = line;
          fragment.append(span);
        } else if (keyword) {
          fragment.append(document.createTextNode(keyword[1]));
          const span = document.createElement("span");
          span.className = "source-keyword";
          span.textContent = keyword[2];
          fragment.append(span, document.createTextNode(keyword[3]));
        } else {
          fragment.append(document.createTextNode(line));
        }
        if (index < sourceLines.length - 1) fragment.append(document.createTextNode("\n"));
      });
      code.replaceChildren(fragment);
      code.dataset.sourceLoaded = "true";
      if (meta) meta.textContent = `exact source · ${sourceLines.length} lines`;
    } catch (_error) {
      code.textContent = "The source preview could not be loaded. Use “Download .scas” above to open it directly.";
      if (meta) meta.textContent = "source preview unavailable";
    } finally {
      code.removeAttribute("aria-busy");
    }
  });
})();
