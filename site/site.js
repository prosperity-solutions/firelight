(function () {
  "use strict";
  ["cmd", "prompt"].forEach(function (id) {
    var pre = document.getElementById(id);
    if (!pre || !navigator.clipboard) return;
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "copy";
    btn.textContent = "Copy";
    btn.setAttribute("aria-label", "Copy " + (id === "cmd" ? "the install command" : "the prompt"));
    btn.addEventListener("click", function () {
      navigator.clipboard.writeText(pre.textContent.trim()).then(function () {
        btn.textContent = "Copied";
        setTimeout(function () { btn.textContent = "Copy"; }, 2000);
      }, function () {
        btn.textContent = "Copy failed";
      });
    });
    pre.parentNode.insertBefore(btn, pre.nextSibling);
  });
})();
