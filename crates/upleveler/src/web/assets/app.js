// Copy buttons: any element with data-copy copies its value and says so briefly.
document.addEventListener("click", (event) => {
  const button = event.target.closest("[data-copy]");
  if (!button || !navigator.clipboard) return;
  navigator.clipboard.writeText(button.dataset.copy).then(() => {
    const text = button.textContent;
    button.textContent = "Copied";
    button.setAttribute("data-copied", "");
    setTimeout(() => {
      button.textContent = text;
      button.removeAttribute("data-copied");
    }, 1500);
  });
});

// Live filters: a GET form with data-live="#target" refreshes that element as
// you type. Without JavaScript the form still works by submitting.
document.querySelectorAll("form[data-live]").forEach((form) => {
  const selector = form.dataset.live;
  let timer;
  let controller;
  const refresh = () => {
    const url = `${form.action}?${new URLSearchParams(new FormData(form))}`;
    controller?.abort();
    controller = new AbortController();
    fetch(url, { signal: controller.signal, headers: { Accept: "text/html" } })
      .then((response) => (response.ok ? response.text() : Promise.reject(response.status)))
      .then((html) => {
        const fresh = new DOMParser().parseFromString(html, "text/html").querySelector(selector);
        const current = document.querySelector(selector);
        if (fresh && current) current.replaceWith(fresh);
        history.replaceState(null, "", url);
      })
      .catch(() => {});
  };
  form.addEventListener("input", () => {
    clearTimeout(timer);
    timer = setTimeout(refresh, 200);
  });
});
