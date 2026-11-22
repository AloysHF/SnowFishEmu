/*
 * SnowFishEmu site interactions: zh/en i18n, the application gallery,
 * animated stats, the hero snow-and-bubbles canvas, and the mobile nav.
 */
(function () {
  'use strict';

  /* ------------------------------------------------------------ i18n */

  var translations = {
    zh: {
      'meta-title': 'SnowFishEmu — 让雪鲤鱼应用在桌面与浏览器重生',
      'meta-desc': 'SnowFishEmu 在桌面与浏览器中运行雪鲤鱼（SnowFish/SFA）功能机 .app 应用：纯 Rust ARM 内核、平台服务桥、RetroArch 核心与在线 WebAssembly 试玩。',
      'nav-features': '核心特性',
      'nav-gallery': '应用画廊',
      'nav-arch': '技术架构',
      'nav-quickstart': '快速开始',
      'nav-play': '在线试玩',
      'hero-title': '让雪鲤鱼应用<br>在<span class="accent">桌面与浏览器</span>重生',
      'hero-lead': 'SnowFishEmu 直接加载 <strong>.app</strong> 可执行文件，在纯 Rust 的 ARM 内核中运行功能机时代的雪鲤鱼（SnowFish/SFA）应用，并补齐它们依赖的手机服务：平台 API、GDI 绘制、文件系统与离线付费应答。',
      'hero-play': '▶ 在线试玩',
      'hero-download': '下载最新版本',
      'phone-soft-left': '选项',
      'phone-soft-right': '开始',
      'about-title': '什么是雪鲤鱼 / SFA 应用？',
      'about-p1': '2000 年代中后期，上海的雪鲤鱼（SnowFish）平台向国产功能机提供可下载的手机应用：<strong>.app</strong> 二进制搭载在 ARM 芯片上运行，通过平台 API 绘制屏幕、读取文件、发起网络与付费请求——那个"KTV 手机小游戏"年代的记忆。',
      'about-p2': 'SnowFishEmu 在没有原厂服务器与运营商短信通道的今天，把这些应用重新跑起来：<strong>纯 Rust 的 ARMv4T CPU 内核</strong>执行来宾代码，<strong>服务桥</strong>逐槽位补齐平台行为，LZ 解压与平台位精确一致。',
      'flow-1-title': '雪鲤鱼平台',
      'flow-1-sub': '功能机时代的应用分发平台（上海雪鲤鱼）',
      'flow-2-sub': 'SFA\\x19 容器 · ARM/Thumb 代码 · 重定位表 · 内嵌资源',
      'flow-3-sub': '桌面窗口 · RetroArch 核心 · 浏览器 WebAssembly',
      'stats-title': '当前进展',
      'stat-1': '语料应用全部可运行',
      'stat-2': 'RGB565 原生分辨率',
      'stat-3': '前端形态（桌面 / RetroArch / 浏览器）',
      'stat-4': '发布构建目标',
      'features-title': '核心特性',
      'features-desc': '从包格式到显示桥，每个层次都按平台行为逐项对齐。',
      'feat-1-title': 'SFA 包格式支持',
      'feat-1-desc': '头部解析、代码重定位、<code>sfa_info</code> 检查工具与姊妹资源文件的自动供给。',
      'feat-2-title': 'ARM/Thumb CPU 内核',
      'feat-2-desc': '纯 Rust 的 ARMv4T 解释器，补全 ARMv5TE 行为（BLX 互通、DSP 乘法），无第三方 C 依赖。',
      'feat-3-title': '平台服务桥',
      'feat-3-desc': '按包装分发点的平台 API 表：日志、来宾文件 I/O、UI 定时器与位精确的 LZ 流解压。',
      'feat-4-title': 'GDI 显示桥',
      'feat-4-desc': '240×320 RGB565 帧缓冲：区域填充、调色/RGB 位块（裁剪与色键透明）、CJK 文本渲染。',
      'feat-5-title': '离线启动与免付费',
      'feat-5-desc': '网络与付费查询本地应答，<code>--free-play</code> 把平台应答为已注册的计费手机，<code>--offline-http</code> 本地完成运营商事务。',
      'feat-6-title': '三种前端',
      'feat-6-desc': '桌面窗口、<code>snowfishemu_libretro</code> 核心（含 SFEMSTAT 存档状态）与浏览器 WebAssembly 核心。',
      'gallery-title': '应用画廊',
      'gallery-desc': '本地验证语料中的全部十款应用。用自己持有的 .app 文件即可在线试玩。',
      'gallery-note': '每款的状态与逐标题操作说明见',
      'chip-sfa': 'sfa 包载入',
      'chip-plat': 'plat 服务桥',
      'chip-gdi': 'gdi 显示桥',
      'chip-lz': 'lz 解压',
      'arch-title': '技术架构',
      'arch-desc': '核心保持平台无关，前端各自负责窗口、输入与呈现。',
      'arch-f1': '桌面窗口前端（minifb）',
      'arch-f2': 'RetroArch / libretro 核心',
      'arch-f3': '浏览器核心（EmulatorJS）',
      'arch-core-sub': '平台无关的模拟器核心',
      'qs-title': '快速开始',
      'qs-desc': '三条路径把 .app 应用跑起来：浏览器、桌面或 RetroArch。',
      'qs-1-title': '在线试玩',
      'qs-1-step1': '打开试玩页面，选择本机的 .app 文件（或拖入页面）。',
      'qs-1-step2': 'WebAssembly 核心随 EmulatorJS 自动加载，文件只在本地处理。',
      'qs-2-title': '桌面运行',
      'qs-2-step1': '从 Releases 下载独立版，或从源码构建：',
      'qs-3-title': 'RetroArch 核心',
      'qs-3-step1': '把核心放入 RetroArch 的 cores/ 目录，info 文件放入 info/。',
      'qs-3-step2': '以 .app 为内容加载即可；方向键 + OK + 软键即是手机键盘。',
      'cta-badge': '无需安装 · 浏览器直接运行',
      'cta-title': '在浏览器里试玩雪鲤鱼游戏',
      'cta-desc': 'WebAssembly 核心随页面加载，选择本地 .app 即刻运行：240×320 画面、手机键盘映射、存档状态保存在浏览器本地。游戏文件不会被上传。',
      'cta-button': '▶ 进入试玩页',
      'footer-blurb': '纯 Rust 的雪鲤鱼（SnowFish/SFA）功能机应用模拟器。本站不分发任何游戏数据，请使用自己合法持有的 .app 文件。',
      'footer-project': '项目',
      'footer-releases': 'Releases 下载',
      'footer-compat': '游戏兼容性',
      'footer-formats': '.app 文件格式',
      'footer-online': '在线体验',
      'footer-play': '在线试玩（EmulatorJS）',
      'footer-license': 'BSD 3-Clause License © 2026 Aloys. Built with 🦀 Rust.',
      'footer-era': '向 2000 年代的功能机小游戏时代致敬',
      'gallery-playable': '可运行'
    },
    en: {
      'meta-title': 'SnowFishEmu — Bring SnowFish apps back on desktop & browser',
      'meta-desc': 'SnowFishEmu runs SnowFish (SFA) feature-phone .app applications on the desktop and in the browser: a pure-Rust ARM core, the platform service bridge, a RetroArch core and online WebAssembly play.',
      'nav-features': 'Features',
      'nav-gallery': 'Gallery',
      'nav-arch': 'Architecture',
      'nav-quickstart': 'Quick Start',
      'nav-play': 'Play Online',
      'hero-title': 'Bring SnowFish apps<br>back on <span class="accent">desktop &amp; browser</span>',
      'hero-lead': 'SnowFishEmu loads <strong>.app</strong> executables directly and runs the mid-2000s SnowFish (SFA) feature-phone applications on a pure-Rust ARM core, bridging the phone services they rely on: the platform API, GDI drawing, the file system and offline billing answers.',
      'hero-play': '▶ Play Online',
      'hero-download': 'Download Latest',
      'phone-soft-left': 'Menu',
      'phone-soft-right': 'Start',
      'about-title': 'What is SnowFish / a SFA application?',
      'about-p1': 'In the mid-to-late 2000s, the Shanghai-based SnowFish platform shipped downloadable applications for domestic feature phones: <strong>.app</strong> binaries ran on ARM chips and drew screens, read files and issued network and billing requests through the platform API — the golden age of feature-phone mini games.',
      'about-p2': 'SnowFishEmu runs those applications today, with no vendor servers or carrier SMS channel: a <strong>pure-Rust ARMv4T CPU core</strong> executes the guest code and a <strong>service bridge</strong> completes the platform behaviour slot by slot, with an LZ decompressor that is bit-exact with the platform.',
      'flow-1-title': 'SnowFish platform',
      'flow-1-sub': 'The mid-2000s application distribution platform (Shanghai SnowFish)',
      'flow-2-sub': 'SFA\\x19 container · ARM/Thumb code · relocation table · embedded resources',
      'flow-3-sub': 'Desktop window · RetroArch core · browser WebAssembly',
      'stats-title': 'Current status',
      'stat-1': 'corpus applications all playable',
      'stat-2': 'native RGB565 resolution',
      'stat-3': 'frontend shapes (desktop / RetroArch / browser)',
      'stat-4': 'release build targets',
      'features-title': 'Core features',
      'features-desc': 'Every layer is aligned against the platform behaviour, from the package format to the display bridge.',
      'feat-1-title': 'SFA package format support',
      'feat-1-desc': 'Header parsing, code relocation, the <code>sfa_info</code> inspector and automatic sibling resource file serving.',
      'feat-2-title': 'ARM/Thumb CPU core',
      'feat-2-desc': 'A pure-Rust ARMv4T interpreter with ARMv5TE completion (BLX interworking, DSP multiplies) — no third-party C dependencies.',
      'feat-3-title': 'Platform service bridge',
      'feat-3-desc': 'The platform API table with wrapper-based dispatch: logging, guest file I/O, the UI timer service and a bit-exact LZ stream decompressor.',
      'feat-4-title': 'GDI display bridge',
      'feat-4-desc': 'A 240×320 RGB565 framebuffer: region fills, palette/RGB blits with source cropping and color-key transparency, and CJK text rendering.',
      'feat-5-title': 'Offline startup & free play',
      'feat-5-desc': 'Network and billing queries are answered locally; <code>--free-play</code> presents a registered billing-capable handset and <code>--offline-http</code> completes carrier transactions locally.',
      'feat-6-title': 'Three frontends',
      'feat-6-desc': 'The desktop window, the <code>snowfishemu_libretro</code> core (with SFEMSTAT save states) and the browser WebAssembly core.',
      'gallery-title': 'Application gallery',
      'gallery-desc': 'All ten applications of the local validation corpus. Use your own .app files to play online.',
      'gallery-note': 'Per-title status and the key recipes are in the',
      'chip-sfa': 'sfa loader',
      'chip-plat': 'plat service bridge',
      'chip-gdi': 'gdi display bridge',
      'chip-lz': 'lz decompressor',
      'arch-title': 'Architecture',
      'arch-desc': 'The core stays platform-independent; each frontend owns its window, input and presentation.',
      'arch-f1': 'Desktop window frontend (minifb)',
      'arch-f2': 'RetroArch / libretro core',
      'arch-f3': 'Browser core (EmulatorJS)',
      'arch-core-sub': 'The platform-independent emulator core',
      'qs-title': 'Quick start',
      'qs-desc': 'Three paths to run a .app application: the browser, the desktop or RetroArch.',
      'qs-1-title': 'Play online',
      'qs-1-step1': 'Open the play page and pick a local .app file (or drop it onto the page).',
      'qs-1-step2': 'The WebAssembly core loads with EmulatorJS; files are processed locally.',
      'qs-2-title': 'Run on the desktop',
      'qs-2-step1': 'Grab the standalone from Releases, or build from source:',
      'qs-3-title': 'RetroArch core',
      'qs-3-step1': 'Put the core into RetroArch\'s cores/ directory and the info file into info/.',
      'qs-3-step2': 'Load a .app as content; the arrows, OK and the soft keys are the phone keypad.',
      'cta-badge': 'No install · runs right in the browser',
      'cta-title': 'Play SnowFish games in the browser',
      'cta-desc': 'The WebAssembly core loads with the page; pick a local .app and it runs immediately: 240×320 video, phone-keypad mapping, save states in browser storage. Game files are never uploaded.',
      'cta-button': '▶ Open the play page',
      'footer-blurb': 'A pure-Rust emulator for SnowFish (SFA) feature-phone applications. This site distributes no game data; use your own legally obtained .app files.',
      'footer-project': 'Project',
      'footer-releases': 'Releases downloads',
      'footer-compat': 'Game compatibility',
      'footer-formats': '.app file formats',
      'footer-online': 'Online',
      'footer-play': 'Play online (EmulatorJS)',
      'footer-license': 'BSD 3-Clause License © 2026 Aloys. Built with 🦀 Rust.',
      'footer-era': 'A tribute to the feature-phone mini-game era of the 2000s',
      'gallery-playable': 'Playable'
    }
  };

  /* --------------------------------------------------------- gallery */

  // The local validation corpus (docs/Game-Compatibility.md). The
  // screenshots are emulator-rendered frame captures committed under
  // docs/images/ (regenerate with scripts/corpus_screenshots.py) and
  // hot-linked from the repository, so the site itself ships no game data.
  var REPO = 'https://raw.githubusercontent.com/AloysHF/SnowFishEmu/master';
  var GAMES = [
    { name: '五子棋', en: 'Gobang', img: '五子棋',
      noteZh: '木纹棋盘与光标可用：方向键移动、OK 落子',
      noteEn: 'The wooden board with a working cursor; arrows move, OK places a stone' },
    { name: '(终身)三国危城无双', en: 'Sanguo: Besieged City', img: '三国危城无双',
      noteZh: '危城龙纹开场，随后进入塔防教程菜单',
      noteEn: 'The dragon-art splash and the tower-defence tutorial menu' },
    { name: '巨作RPG曹操冢', en: 'Caocao Tomb (RPG)', img: '巨作RPG曹操冢',
      noteZh: '游戏进行中，帮助对话框文本可读',
      noteEn: 'Gameplay with the help dialog text readable' },
    { name: '曹操冢超豪华版', en: 'Caocao Tomb Deluxe', img: '曹操冢超豪华版',
      noteZh: '游戏进行中，帮助栏与精灵图页正常',
      noteEn: 'Gameplay; the help bar and sprite pages render' },
    { name: '永恒魔塔', en: 'Eternal Magic Tower', img: '永恒魔塔',
      noteZh: '魔塔楼层渲染，剧情对话完整可读',
      noteEn: 'The magic-tower floor with fully readable story dialog' },
    { name: '胭脂戏游龙', en: 'Rouge Dragon', img: '胭脂戏游龙',
      noteZh: '西门庆居场景、对话框与移动教程正常',
      noteEn: 'The first scene, the dialog box and the movement tutorial' },
    { name: '新星际撞球', en: 'Star Ball', img: '新星际撞球',
      noteZh: '砖块棋盘、球与挡板可玩（计分区有已知的显示小问题）',
      noteEn: 'The board, ball and paddle run; a cosmetic score-strip gap' },
    { name: '新鬼吹灯', en: 'Ghost Blowing Lantern', img: '新鬼吹灯',
      noteZh: '完整标题、剧情过场、菜单与战斗',
      noteEn: 'The full title, story intro, menu and battle' },
    { name: '黄金矿工', en: 'Gold Miner', img: '黄金矿工',
      noteZh: '标题即可开始第一关（离线）',
      noteEn: 'The title starts the first level directly, offline' },
    { name: '宇宙风暴', en: 'Universe Storm', img: '宇宙风暴',
      noteZh: '菜单与第一关可玩，内嵌音乐与位图正常加载',
      noteEn: 'Menu and first level; embedded music and bitmaps load' }
  ];

  function buildGallery(lang) {
    var grid = document.getElementById('gallery-grid');
    if (!grid || grid.childElementCount > 0) return;
    GAMES.forEach(function (game) {
      var card = document.createElement('figure');
      card.className = 'gallery-card';

      var tile = document.createElement('div');
      tile.className = 'gallery-tile';
      var shot = document.createElement('img');
      shot.className = 'gallery-shot';
      shot.loading = 'lazy';
      shot.src = REPO + '/docs/images/' + encodeURIComponent(game.img) + '.png';
      shot.alt = lang === 'en' ? game.en : game.name;
      tile.appendChild(shot);

      var name = document.createElement('figcaption');
      name.className = 'gallery-name';
      name.textContent = lang === 'en' ? game.en : game.name;

      var meta = document.createElement('div');
      meta.className = 'gallery-meta';
      var chip = document.createElement('span');
      chip.className = 'status-chip';
      chip.innerHTML = '<span class="chip-dot"></span>' + (translations[lang] && translations[lang]['gallery-playable']);
      meta.appendChild(chip);

      card.appendChild(tile);
      card.appendChild(name);
      card.appendChild(meta);
      card.title = lang === 'en' ? game.noteEn : game.noteZh;
      grid.appendChild(card);
    });
  }

  /* -------------------------------------------------------- counters */

  function runCounter(el) {
    var target = parseInt(el.getAttribute('data-count'), 10) || 0;
    var suffix = el.getAttribute('data-suffix') || '';
    var duration = 1300;
    var start = null;
    function frame(now) {
      if (start === null) start = now;
      var progress = Math.min((now - start) / duration, 1);
      var eased = 1 - Math.pow(1 - progress, 3);
      el.textContent = Math.round(target * eased) + suffix;
      if (progress < 1) window.requestAnimationFrame(frame);
    }
    window.requestAnimationFrame(frame);
  }

  /* ------------------------------------------------- hero canvas fx */

  function setupHeroCanvas() {
    var canvas = document.getElementById('hero-canvas');
    var hero = canvas && canvas.parentElement;
    if (!canvas || !hero) return;
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;

    var context = canvas.getContext('2d');
    var flakes = [];
    var bubbles = [];

    function resize() {
      var ratio = window.devicePixelRatio || 1;
      var width = hero.clientWidth;
      var height = hero.clientHeight;
      canvas.width = width * ratio;
      canvas.height = height * ratio;
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
    }

    function spawn(count, make) {
      var items = [];
      for (var i = 0; i < count; i += 1) items.push(make(true));
      return items;
    }

    function makeFlake(anywhere) {
      return {
        x: Math.random() * hero.clientWidth,
        y: anywhere ? Math.random() * hero.clientHeight : -6,
        r: 1.2 + Math.random() * 2.4,
        speed: 0.25 + Math.random() * 0.55,
        sway: 0.4 + Math.random() * 0.9,
        phase: Math.random() * Math.PI * 2
      };
    }

    function makeBubble(anywhere) {
      return {
        x: Math.random() * hero.clientWidth,
        y: anywhere ? Math.random() * hero.clientHeight : hero.clientHeight + 10,
        r: 2.5 + Math.random() * 5,
        speed: 0.2 + Math.random() * 0.45,
        wobble: 0.6 + Math.random() * 1.2,
        phase: Math.random() * Math.PI * 2
      };
    }

    flakes = spawn(34, makeFlake);
    bubbles = spawn(12, makeBubble);
    resize();
    window.addEventListener('resize', resize);

    var angle = 0;
    function tick() {
      angle += 0.014;
      var width = hero.clientWidth;
      var height = hero.clientHeight;
      context.clearRect(0, 0, width, height);

      // rising aqua bubbles
      context.strokeStyle = 'rgba(31, 159, 208, 0.22)';
      bubbles.forEach(function (b) {
        b.y -= b.speed;
        b.x += Math.sin(angle * 1.6 + b.phase) * 0.18;
        if (b.y < -12) { b.y = height + 10; b.x = Math.random() * width; }
        context.beginPath();
        context.arc(b.x, b.y, b.r, 0, Math.PI * 2);
        context.stroke();
      });

      // falling soft snow
      context.fillStyle = 'rgba(31, 159, 208, 0.18)';
      flakes.forEach(function (f) {
        f.y += f.speed;
        f.x += Math.sin(angle + f.phase) * f.sway * 0.4;
        if (f.y > height + 8) { f.y = -6; f.x = Math.random() * width; }
        context.beginPath();
        context.arc(f.x, f.y, f.r, 0, Math.PI * 2);
        context.fill();
      });

      window.requestAnimationFrame(tick);
    }
    window.requestAnimationFrame(tick);
  }

  /* ------------------------------------------------------- reveal fx */

  function setupReveal() {
    var items = document.querySelectorAll('.reveal');
    if (!('IntersectionObserver' in window)) {
      items.forEach(function (el) { el.classList.add('visible'); });
      return;
    }
    var observer = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add('visible');
          observer.unobserve(entry.target);
        }
      });
    }, { threshold: 0.12 });
    items.forEach(function (el) { observer.observe(el); });
  }

  function setupCounters() {
    var numbers = document.querySelectorAll('.stat-number');
    if (!('IntersectionObserver' in window)) {
      numbers.forEach(function (el) {
        el.textContent = el.getAttribute('data-count') + (el.getAttribute('data-suffix') || '');
      });
      return;
    }
    var observer = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          runCounter(entry.target);
          observer.unobserve(entry.target);
        }
      });
    }, { threshold: 0.4 });
    numbers.forEach(function (el) { observer.observe(el); });
  }

  /* ----------------------------------------------------------- nav */

  function setupNav() {
    var burger = document.getElementById('nav-burger');
    var links = document.getElementById('nav-links');
    if (!burger || !links) return;
    burger.addEventListener('click', function () {
      var open = links.classList.toggle('open');
      burger.setAttribute('aria-expanded', open ? 'true' : 'false');
    });
    links.addEventListener('click', function (event) {
      if (event.target.tagName === 'A') links.classList.remove('open');
    });
  }

  /* --------------------------------------------------------- boot */

  var storageKey = 'snowfish-site-lang';
  var languageButton = document.getElementById('lang-toggle');
  var description = document.querySelector('meta[name="description"]');
  var currentLang;
  try {
    currentLang = localStorage.getItem(storageKey) || 'zh';
  } catch (error) {
    currentLang = 'zh';
  }

  function applyLang(lang) {
    currentLang = lang;
    document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
    var t = translations[lang];

    document.querySelectorAll('[data-i18n]').forEach(function (el) {
      var key = el.getAttribute('data-i18n');
      if (t[key] !== undefined) el.innerHTML = t[key];
    });

    // The gallery is built once per language switch so the card names
    // follow the dictionary.
    var grid = document.getElementById('gallery-grid');
    if (grid) {
      grid.innerHTML = '';
      buildGallery(lang);
    }

    document.title = t['meta-title'];
    if (description) description.setAttribute('content', t['meta-desc']);
    if (languageButton) {
      languageButton.textContent = lang === 'zh' ? 'EN' : '中';
      languageButton.setAttribute('aria-label', lang === 'zh' ? 'Switch to English' : '切换到中文');
    }

    try {
      localStorage.setItem(storageKey, lang);
    } catch (error) {
      // Language persistence is optional in restricted browsing contexts.
    }
  }

  if (languageButton) {
    languageButton.addEventListener('click', function () {
      applyLang(currentLang === 'zh' ? 'en' : 'zh');
    });
  }

  setupNav();
  setupReveal();
  setupCounters();
  setupHeroCanvas();
  applyLang(currentLang);
})();
