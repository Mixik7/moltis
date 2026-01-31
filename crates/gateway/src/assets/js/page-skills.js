// ── Skills page ─────────────────────────────────────────────
import * as S from "./state.js";
import { sendRpc } from "./helpers.js";
import { registerPage } from "./router.js";

registerPage("/skills", function initSkills(container) {
  container.style.cssText = "flex-direction:column;padding:0;overflow:hidden;";

  var wrapper = document.createElement("div");
  wrapper.className = "flex-1 flex flex-col min-w-0 p-4 gap-4 overflow-y-auto";

  var header = document.createElement("div");
  header.className = "flex items-center gap-3";
  var title = document.createElement("h2");
  title.className = "text-lg font-medium text-[var(--text-strong)]";
  title.textContent = "Skills";
  var refreshBtn = document.createElement("button");
  refreshBtn.textContent = "Refresh";
  refreshBtn.className = "logs-btn";
  header.appendChild(title);
  header.appendChild(refreshBtn);
  wrapper.appendChild(header);

  var desc = document.createElement("p");
  desc.className = "text-sm text-[var(--muted)]";
  desc.textContent = "SKILL.md-based skills discovered from project, personal, and installed paths.";
  wrapper.appendChild(desc);

  // Security warning
  var warnKey = "moltis-skills-warning-dismissed";
  if (!localStorage.getItem(warnKey)) {
    var warn = document.createElement("div");
    warn.className = "skills-warn";
    var warnTitle = document.createElement("div");
    warnTitle.className = "skills-warn-title";
    warnTitle.textContent = "Security Warning: Review skills before installing";
    warn.appendChild(warnTitle);
    var warnIntro = document.createElement("div");
    warnIntro.textContent = "Skills are community-authored instructions that the AI agent follows. A malicious skill can instruct the agent to:";
    warn.appendChild(warnIntro);
    var warnList = document.createElement("ul");
    warnList.style.cssText = "margin:6px 0 6px 18px;padding:0;";
    ["Execute arbitrary shell commands on your machine (install malware, cryptominers, backdoors)",
     "Read and exfiltrate sensitive data \u2014 SSH keys, API tokens, browser cookies, credentials, env variables",
     "Modify or delete files across your filesystem, including other projects",
     "Send your data to remote servers via curl/wget without your knowledge"
    ].forEach(function (t) { var li = document.createElement("li"); li.textContent = t; warnList.appendChild(li); });
    warn.appendChild(warnList);
    var warnAdvice = document.createElement("div");
    warnAdvice.style.cssText = "margin-top:4px;";
    warnAdvice.textContent = "Only install skills from authors and repositories you trust. Always read the full SKILL.md before enabling a skill \u2014 the instructions in the body are what the agent will execute.";
    var warnSandbox = document.createElement("div");
    warnSandbox.style.cssText = "margin-top:6px;color:var(--success, #4a4);";
    warnSandbox.textContent = "With sandbox mode enabled (Docker, Apple Container, or cgroup), command execution is isolated and the damage a malicious skill can do is significantly limited.";
    warn.appendChild(warnAdvice); warn.appendChild(warnSandbox);
    var warnDismiss = document.createElement("button");
    warnDismiss.textContent = "Dismiss";
    warnDismiss.style.cssText = "margin-top:8px;background:none;border:1px solid var(--border);border-radius:var(--radius-sm);font-size:.72rem;padding:3px 10px;cursor:pointer;color:var(--muted);";
    warnDismiss.addEventListener("click", function () { localStorage.setItem(warnKey, "1"); warn.remove(); });
    warn.appendChild(warnDismiss);
    wrapper.appendChild(warn);
  }

  // Toast
  var toastContainer = document.createElement("div");
  toastContainer.className = "skills-toast-container";
  document.body.appendChild(toastContainer);

  function showToast(message, type) {
    var bgColor = type === "error" ? "var(--error, #e55)" : "var(--accent)";
    var toast = document.createElement("div");
    toast.style.cssText = "pointer-events:auto;max-width:420px;padding:10px 16px;border-radius:6px;font-size:.8rem;font-weight:500;color:#fff;background:" + bgColor + ";box-shadow:0 4px 12px rgba(0,0,0,.15);opacity:0;transform:translateY(-8px);transition:opacity .2s,transform .2s;";
    toast.textContent = message;
    toastContainer.appendChild(toast);
    requestAnimationFrame(function () { toast.style.opacity = "1"; toast.style.transform = "translateY(0)"; });
    setTimeout(function () { toast.style.opacity = "0"; toast.style.transform = "translateY(-8px)"; setTimeout(function () { toast.remove(); }, 200); }, 4000);
  }

  // Install form
  var installBox = document.createElement("div");
  installBox.className = "skills-install-box";
  var installInput = document.createElement("input");
  installInput.type = "text";
  installInput.placeholder = "owner/repo or full URL (e.g. anthropics/skills)";
  installInput.className = "skills-install-input";
  var installBtn = document.createElement("button");
  installBtn.textContent = "Install";
  installBtn.className = "skills-install-btn";
  installBox.appendChild(installInput); installBox.appendChild(installBtn);
  wrapper.appendChild(installBox);

  function doInstall(source, btn) {
    if (!source) return;
    if (!S.connected) { showToast("Not connected to gateway.", "error"); return; }
    var origText = btn.textContent;
    btn.textContent = "Installing\u2026"; btn.disabled = true; btn.style.opacity = ".6";
    sendRpc("skills.install", { source: source }).then(function (res) {
      btn.textContent = origText; btn.disabled = false; btn.style.opacity = "";
      if (res && res.ok) {
        var p = res.payload || {};
        var count = (p.installed || []).length;
        showToast("Installed " + source + " (" + count + " skill" + (count !== 1 ? "s" : "") + ")", "success");
        fetchAll();
      } else { showToast("Failed: " + (res && res.error || "unknown error"), "error"); }
    });
  }

  installBtn.addEventListener("click", function () { doInstall(installInput.value.trim(), installBtn); });
  installInput.addEventListener("keydown", function (e) { if (e.key === "Enter") doInstall(installInput.value.trim(), installBtn); });

  // Featured skills
  var featuredSkills = [
    { repo: "openclaw/skills", desc: "Community skills from ClawdHub" },
    { repo: "anthropics/skills", desc: "Official Anthropic agent skills" },
    { repo: "vercel-labs/agent-skills", desc: "Vercel agent skills collection" },
    { repo: "vercel-labs/skills", desc: "Vercel skills toolkit" }
  ];
  var featuredSection = document.createElement("div");
  featuredSection.className = "skills-section";
  var featuredTitle = document.createElement("h3");
  featuredTitle.className = "skills-section-title";
  featuredTitle.textContent = "Featured Skills";
  featuredSection.appendChild(featuredTitle);
  var featuredGrid = document.createElement("div");
  featuredGrid.className = "skills-featured-grid";
  featuredSkills.forEach(function (f) {
    var card = document.createElement("div");
    card.className = "skills-featured-card";
    var info = document.createElement("div");
    var repoName = document.createElement("a");
    repoName.style.cssText = "font-family:var(--font-mono);font-size:.82rem;font-weight:500;color:var(--text-strong);text-decoration:none;";
    repoName.textContent = f.repo;
    repoName.href = /^https?:\/\//.test(f.repo) ? f.repo : "https://github.com/" + f.repo;
    repoName.target = "_blank"; repoName.rel = "noopener noreferrer";
    repoName.onmouseenter = function () { repoName.style.textDecoration = "underline"; };
    repoName.onmouseleave = function () { repoName.style.textDecoration = "none"; };
    var repoDesc = document.createElement("div");
    repoDesc.style.cssText = "font-size:.75rem;color:var(--muted);";
    repoDesc.textContent = f.desc;
    info.appendChild(repoName); info.appendChild(repoDesc); card.appendChild(info);
    var btn = document.createElement("button");
    btn.textContent = "Install";
    btn.style.cssText = "background:var(--surface2);border:1px solid var(--border);color:var(--text);border-radius:var(--radius-sm);font-size:.72rem;padding:4px 10px;cursor:pointer;white-space:nowrap;";
    btn.addEventListener("click", function () { doInstall(f.repo, btn); });
    card.appendChild(btn); featuredGrid.appendChild(card);
  });
  featuredSection.appendChild(featuredGrid);
  wrapper.appendChild(featuredSection);

  // Repos section
  var reposSection = document.createElement("div");
  reposSection.className = "skills-section";
  var reposTitle = document.createElement("h3");
  reposTitle.className = "skills-section-title";
  reposTitle.textContent = "Installed Repositories";
  reposSection.appendChild(reposTitle);
  var reposWrap = document.createElement("div");
  reposWrap.className = "skills-section";
  reposSection.appendChild(reposWrap);
  wrapper.appendChild(reposSection);

  // Enabled skills table
  var skillsTitle = document.createElement("h3");
  skillsTitle.className = "skills-section-title";
  skillsTitle.textContent = "Enabled Skills";
  wrapper.appendChild(skillsTitle);
  var tableWrap = document.createElement("div");
  tableWrap.className = "skills-table-wrap";
  wrapper.appendChild(tableWrap);
  container.appendChild(wrapper);

  function renderRepos(repos) {
    reposWrap.textContent = "";
    if (!repos || repos.length === 0) {
      var empty = document.createElement("div");
      empty.style.cssText = "padding:12px;color:var(--muted);font-size:.82rem;";
      empty.textContent = "No repositories installed.";
      reposWrap.appendChild(empty);
      return;
    }
    repos.forEach(function (repo) {
      var card = document.createElement("div");
      card.className = "skills-repo-card";
      var hdr = document.createElement("div");
      hdr.className = "skills-repo-header";
      var hdrLeft = document.createElement("div");
      hdrLeft.style.cssText = "display:flex;align-items:center;gap:8px;";
      var arrow = document.createElement("span");
      arrow.textContent = "\u25B6";
      arrow.style.cssText = "font-size:.65rem;color:var(--muted);transition:transform .15s;";
      var repoNameEl = document.createElement("a");
      repoNameEl.style.cssText = "font-family:var(--font-mono);font-size:.82rem;font-weight:500;color:var(--text-strong);text-decoration:none;";
      repoNameEl.textContent = repo.source;
      repoNameEl.href = /^https?:\/\//.test(repo.source) ? repo.source : "https://github.com/" + repo.source;
      repoNameEl.target = "_blank"; repoNameEl.rel = "noopener noreferrer";
      repoNameEl.addEventListener("click", function (e) { e.stopPropagation(); });
      repoNameEl.onmouseenter = function () { repoNameEl.style.textDecoration = "underline"; };
      repoNameEl.onmouseleave = function () { repoNameEl.style.textDecoration = "none"; };
      var skillCount = document.createElement("span");
      skillCount.style.cssText = "font-size:.72rem;color:var(--muted);";
      var enabledCount = (repo.skills || []).filter(function (s) { return s.enabled; }).length;
      skillCount.textContent = enabledCount + "/" + (repo.skills || []).length + " enabled";
      hdrLeft.appendChild(arrow); hdrLeft.appendChild(repoNameEl); hdrLeft.appendChild(skillCount);
      hdr.appendChild(hdrLeft);

      var rmBtn = document.createElement("button");
      rmBtn.textContent = "Remove Repo";
      rmBtn.style.cssText = "background:none;border:1px solid var(--border);color:var(--error, #e55);border-radius:var(--radius-sm);font-size:.72rem;padding:3px 8px;cursor:pointer;";
      rmBtn.addEventListener("click", function (e) {
        e.stopPropagation();
        if (!S.connected) return;
        sendRpc("skills.repos.remove", { source: repo.source }).then(function (res) { if (res && res.ok) fetchAll(); });
      });
      hdr.appendChild(rmBtn);
      card.appendChild(hdr);

      var detail = document.createElement("div");
      detail.className = "skills-repo-detail";
      var expanded = false;
      hdr.addEventListener("click", function () {
        expanded = !expanded;
        detail.style.display = expanded ? "block" : "none";
        arrow.style.transform = expanded ? "rotate(90deg)" : "";
      });

      var searchRow = document.createElement("div");
      searchRow.style.cssText = "position:relative;margin-bottom:8px;";
      var repoSearchInput = document.createElement("input");
      repoSearchInput.type = "text";
      repoSearchInput.placeholder = "Search skills in " + repo.source + "\u2026";
      repoSearchInput.style.cssText = "width:100%;padding:6px 10px;border:1px solid var(--border);border-radius:var(--radius-sm);background:var(--surface);color:var(--text);font-size:.8rem;font-family:var(--font-mono);box-sizing:border-box;";
      searchRow.appendChild(repoSearchInput);
      var acDrop = document.createElement("div");
      acDrop.className = "skills-ac-dropdown";
      searchRow.appendChild(acDrop);
      detail.appendChild(searchRow);

      var detailPanel = document.createElement("div");
      detailPanel.className = "skills-detail-panel";
      detail.appendChild(detailPanel);

      var allSkills = repo.skills || [];

      (function (allSkills, repo, searchInput, acDrop, detailPanel) {
        var acIdx = -1;

        function renderAcResults(query) {
          acDrop.textContent = ""; acIdx = -1;
          if (!query) { acDrop.style.display = "none"; return; }
          var q = query.toLowerCase();
          var matches = allSkills.filter(function (s) {
            return s.name.toLowerCase().indexOf(q) !== -1 || (s.display_name || "").toLowerCase().indexOf(q) !== -1 || (s.description || "").toLowerCase().indexOf(q) !== -1;
          }).slice(0, 30);
          if (matches.length === 0) {
            var noRes = document.createElement("div");
            noRes.style.cssText = "padding:8px 10px;color:var(--muted);font-size:.78rem;";
            noRes.textContent = "No matching skills.";
            acDrop.appendChild(noRes); acDrop.style.display = "block"; return;
          }
          matches.forEach(function (skill) {
            var item = document.createElement("div");
            item.className = "skills-ac-item";
            var left = document.createElement("div");
            left.style.cssText = "display:flex;align-items:center;gap:6px;min-width:0;";
            var nm = document.createElement("span");
            nm.style.cssText = "font-family:var(--font-mono);font-weight:500;color:var(--text-strong);white-space:nowrap;";
            nm.textContent = skill.display_name || skill.name;
            left.appendChild(nm);
            if (skill.display_name) {
              var slug = document.createElement("span");
              slug.style.cssText = "color:var(--muted);font-size:.68rem;font-family:var(--font-mono);white-space:nowrap;";
              slug.textContent = skill.name; left.appendChild(slug);
            }
            if (skill.description) {
              var ds = document.createElement("span");
              ds.style.cssText = "color:var(--muted);font-size:.72rem;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;";
              ds.textContent = skill.description; left.appendChild(ds);
            }
            item.appendChild(left);
            var badges = document.createElement("div");
            badges.style.cssText = "display:flex;align-items:center;gap:4px;flex-shrink:0;margin-left:8px;";
            if (skill.enabled) {
              var enBadge = document.createElement("span");
              enBadge.style.cssText = "font-size:.6rem;padding:1px 5px;border-radius:9999px;background:var(--accent);color:#fff;font-weight:500;";
              enBadge.textContent = "enabled"; badges.appendChild(enBadge);
            }
            if (skill.eligible === false) {
              var blk = document.createElement("span");
              blk.style.cssText = "font-size:.6rem;padding:1px 5px;border-radius:9999px;background:var(--error, #e55);color:#fff;font-weight:500;";
              blk.textContent = "blocked"; badges.appendChild(blk);
            }
            item.appendChild(badges);
            item.addEventListener("click", function () { searchInput.value = skill.name; acDrop.style.display = "none"; showSkillDetail(skill); });
            acDrop.appendChild(item);
          });
          acDrop.style.display = "block";
        }

        function showSkillDetail(skill) {
          detailPanel.textContent = ""; detailPanel.style.display = "block";
          var loadMsg = document.createElement("div");
          loadMsg.style.cssText = "color:var(--muted);font-size:.8rem;";
          loadMsg.textContent = "Loading\u2026";
          detailPanel.appendChild(loadMsg);
          sendRpc("skills.skill.detail", { source: repo.source, skill: skill.name }).then(function (res) {
            detailPanel.textContent = "";
            if (!res || !res.ok) {
              var err = document.createElement("div");
              err.style.cssText = "color:var(--error, #e55);font-size:.8rem;";
              err.textContent = "Failed to load: " + (res && res.error || "unknown");
              detailPanel.appendChild(err); return;
            }
            var d = res.payload || {};
            var dHdr = document.createElement("div");
            dHdr.style.cssText = "display:flex;align-items:center;justify-content:space-between;margin-bottom:8px;";
            var dLeft = document.createElement("div");
            dLeft.style.cssText = "display:flex;align-items:center;gap:8px;";
            var dName = document.createElement("span");
            dName.style.cssText = "font-family:var(--font-mono);font-size:.9rem;font-weight:600;color:var(--text-strong);";
            dName.textContent = d.display_name || d.name;
            dLeft.appendChild(dName);
            if (d.display_name) {
              var dSlug = document.createElement("span");
              dSlug.style.cssText = "font-family:var(--font-mono);font-size:.72rem;color:var(--muted);";
              dSlug.textContent = d.name; dLeft.appendChild(dSlug);
            }
            if (d.license) {
              var licBadge = document.createElement("span");
              licBadge.style.cssText = "font-size:.65rem;padding:1px 6px;border-radius:9999px;background:var(--surface2);color:var(--muted);";
              licBadge.textContent = d.license; dLeft.appendChild(licBadge);
            }
            var hasReqs = d.requires && ((d.requires.bins && d.requires.bins.length) || (d.requires.any_bins && d.requires.any_bins.length));
            if (d.eligible === false) {
              var blkB = document.createElement("span");
              blkB.style.cssText = "font-size:.65rem;padding:1px 5px;border-radius:9999px;background:var(--error, #e55);color:#fff;font-weight:500;";
              blkB.textContent = "blocked"; dLeft.appendChild(blkB);
            } else if (hasReqs) {
              var okB = document.createElement("span");
              okB.style.cssText = "font-size:.65rem;padding:1px 5px;border-radius:9999px;background:var(--success, #4a4);color:#fff;font-weight:500;";
              okB.textContent = "eligible"; dLeft.appendChild(okB);
            } else {
              var unkB = document.createElement("span");
              unkB.style.cssText = "font-size:.65rem;padding:1px 5px;border-radius:9999px;background:var(--surface2);color:var(--muted);font-weight:500;";
              unkB.textContent = "no deps declared"; dLeft.appendChild(unkB);
            }
            dHdr.appendChild(dLeft);
            var dRight = document.createElement("div");
            dRight.style.cssText = "display:flex;align-items:center;gap:6px;";
            var toggleBtn = document.createElement("button");
            toggleBtn.textContent = d.enabled ? "Disable" : "Enable";
            toggleBtn.style.cssText = "background:" + (d.enabled ? "none" : "var(--accent)") + ";border:1px solid var(--border);border-radius:var(--radius-sm);font-size:.72rem;padding:3px 10px;cursor:pointer;color:" + (d.enabled ? "var(--muted)" : "#fff") + ";font-weight:500;";
            toggleBtn.addEventListener("click", function () {
              if (!S.connected) return;
              var method = d.enabled ? "skills.skill.disable" : "skills.skill.enable";
              sendRpc(method, { source: repo.source, skill: d.name }).then(function (r) { if (r && r.ok) fetchAll(); });
            });
            dRight.appendChild(toggleBtn);
            var closeBtn = document.createElement("button");
            closeBtn.textContent = "\u2715";
            closeBtn.style.cssText = "background:none;border:none;color:var(--muted);font-size:.9rem;cursor:pointer;padding:2px 4px;";
            closeBtn.addEventListener("click", function () { detailPanel.style.display = "none"; searchInput.value = ""; });
            dRight.appendChild(closeBtn);
            dHdr.appendChild(dRight);
            detailPanel.appendChild(dHdr);

            var metaParts = [];
            if (d.author) metaParts.push("Author: " + d.author);
            if (d.version) metaParts.push("v" + d.version);
            if (metaParts.length || d.homepage || d.source_url) {
              var metaRow = document.createElement("div");
              metaRow.style.cssText = "display:flex;align-items:center;gap:12px;margin-bottom:8px;font-size:.75rem;color:var(--muted);flex-wrap:wrap;";
              metaParts.forEach(function (txt) { var sp = document.createElement("span"); sp.textContent = txt; metaRow.appendChild(sp); });
              if (d.homepage) {
                var hpLink = document.createElement("a");
                hpLink.href = d.homepage; hpLink.target = "_blank"; hpLink.rel = "noopener noreferrer";
                hpLink.style.cssText = "color:var(--accent);text-decoration:none;font-size:.75rem;";
                hpLink.textContent = d.homepage.replace(/^https?:\/\//, "");
                hpLink.onmouseenter = function () { hpLink.style.textDecoration = "underline"; };
                hpLink.onmouseleave = function () { hpLink.style.textDecoration = "none"; };
                metaRow.appendChild(hpLink);
              }
              if (d.source_url) {
                var srcLink = document.createElement("a");
                srcLink.href = d.source_url; srcLink.target = "_blank"; srcLink.rel = "noopener noreferrer";
                srcLink.style.cssText = "color:var(--accent);text-decoration:none;font-size:.75rem;";
                srcLink.textContent = "View source";
                srcLink.onmouseenter = function () { srcLink.style.textDecoration = "underline"; };
                srcLink.onmouseleave = function () { srcLink.style.textDecoration = "none"; };
                metaRow.appendChild(srcLink);
              }
              detailPanel.appendChild(metaRow);
            }
            if (d.description) {
              var dDesc = document.createElement("p");
              dDesc.style.cssText = "margin:0 0 8px;font-size:.82rem;color:var(--text);";
              dDesc.textContent = d.description; detailPanel.appendChild(dDesc);
            }
            if (d.eligible === false && d.missing_bins && d.missing_bins.length) {
              var missingDiv = document.createElement("div");
              missingDiv.style.cssText = "margin-bottom:8px;font-size:.78rem;";
              var missingLabel = document.createElement("span");
              missingLabel.style.cssText = "color:var(--error, #e55);font-weight:500;";
              missingLabel.textContent = "Missing: " + d.missing_bins.map(function (b) { return "bin:" + b; }).join(", ");
              missingDiv.appendChild(missingLabel);
              (d.install_options || []).forEach(function (opt, idx) {
                var iBtn = document.createElement("button");
                iBtn.textContent = opt.label || ("Install via " + opt.kind);
                iBtn.style.cssText = "margin-left:6px;background:var(--accent);color:#fff;border:none;border-radius:var(--radius-sm);font-size:.7rem;padding:2px 8px;cursor:pointer;";
                iBtn.addEventListener("click", function () {
                  iBtn.textContent = "Installing\u2026"; iBtn.disabled = true; iBtn.style.opacity = ".6";
                  sendRpc("skills.install_dep", { skill: d.name, index: idx }).then(function (r) {
                    if (r && r.ok) { showToast("Installed dependency for " + d.name, "success"); showSkillDetail(skill); }
                    else { iBtn.textContent = opt.label || ("Install via " + opt.kind); iBtn.disabled = false; iBtn.style.opacity = ""; showToast("Install failed: " + (r && r.error || "unknown"), "error"); }
                  });
                });
                missingDiv.appendChild(iBtn);
              });
              detailPanel.appendChild(missingDiv);
            }
            if (d.compatibility) {
              var compatDiv = document.createElement("div");
              compatDiv.style.cssText = "margin-bottom:8px;font-size:.75rem;color:var(--muted);font-style:italic;";
              compatDiv.textContent = d.compatibility; detailPanel.appendChild(compatDiv);
            }
            if (d.allowed_tools && d.allowed_tools.length) {
              var toolsDiv = document.createElement("div");
              toolsDiv.style.cssText = "margin-bottom:8px;font-size:.75rem;color:var(--muted);";
              toolsDiv.textContent = "Allowed tools: " + d.allowed_tools.join(", "); detailPanel.appendChild(toolsDiv);
            }
            if (d.body_html) {
              var bodyDiv = document.createElement("div");
              bodyDiv.className = "skill-body-md";
              bodyDiv.style.cssText = "border-top:1px solid var(--border);padding-top:8px;margin-top:8px;max-height:400px;overflow-y:auto;font-size:.8rem;color:var(--text);line-height:1.5;";
              bodyDiv.innerHTML = d.body_html;
              bodyDiv.querySelectorAll("a").forEach(function (a) { a.setAttribute("target", "_blank"); a.setAttribute("rel", "noopener"); });
              detailPanel.appendChild(bodyDiv);
            } else if (d.body) {
              var bodyDiv2 = document.createElement("div");
              bodyDiv2.style.cssText = "border-top:1px solid var(--border);padding-top:8px;margin-top:8px;";
              var pre = document.createElement("pre");
              pre.style.cssText = "white-space:pre-wrap;word-break:break-word;font-size:.78rem;color:var(--text);font-family:var(--font-mono);margin:0;max-height:400px;overflow-y:auto;";
              pre.textContent = d.body; bodyDiv2.appendChild(pre); detailPanel.appendChild(bodyDiv2);
            }
          });
        }

        searchInput.addEventListener("input", function () { renderAcResults(searchInput.value.trim()); });
        searchInput.addEventListener("keydown", function (e) {
          var items = acDrop.querySelectorAll("[style*='cursor:pointer']");
          if (e.key === "ArrowDown") { e.preventDefault(); acIdx = Math.min(acIdx + 1, items.length - 1); items.forEach(function (it, i) { it.style.background = i === acIdx ? "var(--bg-hover)" : ""; }); }
          else if (e.key === "ArrowUp") { e.preventDefault(); acIdx = Math.max(acIdx - 1, 0); items.forEach(function (it, i) { it.style.background = i === acIdx ? "var(--bg-hover)" : ""; }); }
          else if (e.key === "Enter" && acIdx >= 0 && items[acIdx]) { e.preventDefault(); items[acIdx].click(); }
          else if (e.key === "Escape") { acDrop.style.display = "none"; }
        });
        document.addEventListener("click", function (e) { if (!searchRow.contains(e.target)) acDrop.style.display = "none"; });
      })(allSkills, repo, repoSearchInput, acDrop, detailPanel);

      card.appendChild(detail);
      reposWrap.appendChild(card);
    });
  }

  var skillRepoMap = {};

  function renderSkills(skills, repos) {
    skillRepoMap = {};
    (repos || []).forEach(function (repo) {
      (repo.skills || []).forEach(function (s) { if (s.enabled) skillRepoMap[s.name] = repo.source; });
    });
    tableWrap.textContent = "";
    if (!skills || skills.length === 0) {
      var empty = document.createElement("div");
      empty.style.cssText = "padding:24px;text-align:center;color:var(--muted);font-size:.85rem;";
      empty.textContent = "No skills found. Install a skill above or add SKILL.md files to .moltis/skills/.";
      tableWrap.appendChild(empty); return;
    }
    var table = document.createElement("table");
    table.style.cssText = "width:100%;border-collapse:collapse;font-size:.82rem;";
    var thead = document.createElement("thead");
    var headRow = document.createElement("tr");
    headRow.style.cssText = "border-bottom:1px solid var(--border);background:var(--surface);";
    ["Name", "Description", ""].forEach(function (h) {
      var th = document.createElement("th");
      th.style.cssText = "text-align:left;padding:8px 12px;font-weight:500;color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.04em;";
      th.textContent = h; headRow.appendChild(th);
    });
    thead.appendChild(headRow); table.appendChild(thead);
    var tbody = document.createElement("tbody");
    skills.forEach(function (s) {
      var row = document.createElement("tr");
      row.style.cssText = "border-bottom:1px solid var(--border);";
      row.onmouseenter = function () { row.style.background = "var(--bg-hover)"; };
      row.onmouseleave = function () { row.style.background = ""; };
      var nameCell = document.createElement("td");
      nameCell.style.cssText = "padding:8px 12px;font-weight:500;color:var(--text-strong);font-family:var(--font-mono);";
      nameCell.textContent = s.name; row.appendChild(nameCell);
      var descCell = document.createElement("td");
      descCell.style.cssText = "padding:8px 12px;color:var(--text);";
      descCell.textContent = s.description || "\u2014"; row.appendChild(descCell);
      var actCell = document.createElement("td");
      actCell.style.cssText = "padding:8px 12px;text-align:right;";
      var repoSource = skillRepoMap[s.name];
      if (repoSource) {
        var disBtn = document.createElement("button");
        disBtn.textContent = "Disable";
        disBtn.style.cssText = "background:none;border:1px solid var(--border);border-radius:var(--radius-sm);font-size:.72rem;padding:2px 8px;cursor:pointer;color:var(--muted);";
        disBtn.addEventListener("click", function () {
          if (!S.connected) return;
          sendRpc("skills.skill.disable", { source: repoSource, skill: s.name }).then(function (res) { if (res && res.ok) fetchAll(); });
        });
        actCell.appendChild(disBtn);
      }
      row.appendChild(actCell); tbody.appendChild(row);
    });
    table.appendChild(tbody); tableWrap.appendChild(table);
  }

  var cachedRepos = [];
  function fetchAll() {
    if (!S.connected) return;
    sendRpc("skills.repos.list", {}).then(function (res) {
      if (res && res.ok) { cachedRepos = res.payload || []; renderRepos(cachedRepos); }
      fetchSkills();
    });
  }
  function fetchSkills() {
    tableWrap.textContent = "";
    var loading = document.createElement("div");
    loading.style.cssText = "padding:24px;text-align:center;color:var(--muted);font-size:.85rem;";
    loading.textContent = "Loading skills\u2026"; tableWrap.appendChild(loading);
    if (!S.connected) { loading.textContent = "Not connected to gateway."; return; }
    sendRpc("skills.list", {}).then(function (res) {
      if (res && res.ok) { renderSkills(res.payload || [], cachedRepos); }
      else { loading.textContent = "Failed to load skills."; }
    });
  }

  refreshBtn.addEventListener("click", fetchAll);
  fetchAll();
});
