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
})();
