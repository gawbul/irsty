// The docs' title links to the site's front page, one level above the book.
// path_to_root is mdBook's: the way from this page to the book's root.
"use strict";

(() => {
  const title = document.querySelector(".menu-title");
  if (!title || typeof path_to_root !== "string") return;
  const link = document.createElement("a");
  link.href = `${path_to_root}../`;
  link.textContent = title.textContent;
  link.title = "Irsty's home page";
  title.replaceChildren(link);
})();
