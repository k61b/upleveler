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
