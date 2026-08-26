;(() => {
  const SEEK_STEP = 5;

  let seeking = false;
  let savedConfig = null;
  let boundHls = null;

  const get_hls = () => window.__chzzkCore?.player?._mediaController?._hls;

  const get_video = () => {
    const root =
      document.getElementById("live_player_layout") ||
      document.getElementById("player_layout");
    return root?.querySelector("video") ?? null;
  };

  const patch_hls = (hls) => {
    hls.streamController.synchronizeToLiveEdge = function (levelDetails) {
      const { config, media } = this;
      if (!media) return;

      const liveSyncPosition = this.hls.liveSyncPosition;
      const currentTime = seeking ? levelDetails.edge : media.currentTime;
      const start = levelDetails.fragments[0].start;
      const end = levelDetails.edge;
      const withinSlidingWindow =
        currentTime >= start - config.maxFragLookUpTolerance &&
        currentTime <= end;

      if (
        liveSyncPosition === null ||
        media.duration <= liveSyncPosition ||
        (currentTime >= liveSyncPosition && withinSlidingWindow)
      ) {
        return;
      }

      const maxLatency =
        config.liveMaxLatencyDuration !== undefined
          ? config.liveMaxLatencyDuration
          : config.liveMaxLatencyDurationCount * levelDetails.targetduration;

      if (
        (!withinSlidingWindow && media.readyState < 4) ||
        currentTime < end - maxLatency
      ) {
        if (!this.loadedmetadata) this.nextLoadPosition = liveSyncPosition;
        if (media.readyState && !seeking) media.currentTime = liveSyncPosition;
      }
    };
  };

  const set_seeking = (value) => {
    const hls = get_hls();
    if (!hls?.config || !hls.streamController) return;

    if (hls !== boundHls) {
      boundHls = hls;
      seeking = false;
      savedConfig = null;
      patch_hls(hls);
    }
    if (seeking === value) return;

    seeking = value;

    if (value) {
      savedConfig = {
        backBufferLength: hls.config.backBufferLength,
        maxBufferSize: hls.config.maxBufferSize,
        maxBufferLength: hls.config.maxBufferLength,
        maxMaxBufferLength: hls.config.maxMaxBufferLength,
      };
      hls.config.backBufferLength = Infinity;
      hls.config.maxBufferSize *= 1000;
      hls.config.maxBufferLength *= 1000;
      hls.config.maxMaxBufferLength *= 1000;
    } else if (savedConfig) {
      Object.assign(hls.config, savedConfig);
      savedConfig = null;
    }
  };

  const seek = (backward) => {
    const video = get_video();
    if (!video || video.duration !== Infinity || video.buffered.length === 0) {
      return false;
    }

    const bufferStart = video.buffered.start(0);
    const bufferEnd = video.buffered.end(video.buffered.length - 1);

    if (backward) {
      video.currentTime = Math.max(
        bufferStart + 1,
        video.currentTime - SEEK_STEP,
      );
      set_seeking(true);
    } else if (video.currentTime + SEEK_STEP >= bufferEnd) {
      video.currentTime = bufferEnd - 1;
      set_seeking(false);
    } else {
      video.currentTime += SEEK_STEP;
      set_seeking(true);
    }
    return true;
  };

  const go_live = () => {
    const video = get_video();
    if (!video || video.buffered.length === 0) return;
    video.currentTime = video.buffered.end(video.buffered.length - 1);
    set_seeking(false);
  };

  const get_vue_constructor = () => {
    let vue = document.querySelector(".pzp-pc")?.__vue__;
    while (vue != null && !Object.hasOwn(vue, "$mount")) {
      vue = Object.getPrototypeOf(vue);
    }
    return vue?.constructor;
  };

  const mount_go_live_button = () => {
    if (!document.getElementById("live_player_layout")) return;

    const pzp = document.querySelector(".pzp-pc");
    const playButton = pzp?.querySelector(".pzp-pc__playback-switch");
    if (!playButton || pzp.querySelector(".chzzk-go-live")) return;

    const Vue = get_vue_constructor();
    if (!Vue) return;

    try {
      const button = new Vue({
        template:
          `<pzp-pc-ui-button class="pzp-pc__playback-switch chzzk-go-live"` +
          ` label="빨리 감기" aria-label="빨리 감기" tooltip="빨리 감기"` +
          ` @click="go_live"><ui-next-media-icon /></pzp-pc-ui-button>`,
        methods: { go_live },
      });
      button.$mount();
      playButton.insertAdjacentElement("afterend", button.$el);
    } catch {}
  };

  const is_typing = (target) =>
    target?.isContentEditable ||
    ["input", "textarea"].includes(target?.tagName?.toLowerCase());

  const sync_live_edge = () => {
    const video = get_video();
    if (!video || video.duration !== Infinity || video.buffered.length === 0) {
      return;
    }
    if (video.buffered.end(video.buffered.length - 1) - video.currentTime < 2) {
      set_seeking(false);
    }
  };

  document.addEventListener("seeked", sync_live_edge, true);

  mount_go_live_button();
  setInterval(mount_go_live_button, 1000);

  document.addEventListener(
    "keydown",
    (event) => {
      const backward = event.key === "ArrowLeft";
      if (!backward && event.key !== "ArrowRight") return;
      if (event.shiftKey || event.altKey || event.metaKey || event.ctrlKey) return;
      if (is_typing(event.target) || !document.hasFocus()) return;

      if (seek(backward)) {
        event.preventDefault();
        event.stopPropagation();
      }
    },
    true,
  );
})();
