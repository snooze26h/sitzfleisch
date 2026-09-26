export function describeStatus(state) {
  if (state.connection === "disconnected") {
    return {
      title: "未连接 · 已解除屏蔽",
      detail: "无法连接坐功，当前已停止屏蔽。重新打开主程序后会按学习日状态同步。"
        + (state.error === "apply-error" ? "部分旧屏蔽页未能恢复，请刷新页面。" : "")
        + (state.error === "release-save-error" ? "解除状态未能写入缓存，本次仍已解除。" : ""),
    };
  }
  const counts = state.hostCount > 0
    ? `${state.hostCount} 条整站规则、${state.urlCount} 条精确网址规则`
    : `${state.ruleCount} 条完整网址规则`;
  if (state.connection === "connected") {
    return {
      title: state.active ? "已连接 · 学习日屏蔽中" : "已连接 · 当前未屏蔽",
      detail: (state.active ? `已应用 ${counts}。暂停和休息时继续生效。` : "主程序当前未启用网站屏蔽。")
        + (state.supportsHosts === false ? "主程序版本较旧，整站规则尚未同步到扩展，请更新主程序。" : ""),
    };
  }
  const reason = state.error === "response-error" ? "主程序返回的规则无效。"
    : state.error === "apply-error" ? "规则应用未完成，请重试同步。"
      : "暂时无法连接主程序。";
  return {
    title: state.connection === "cache" ? "同步异常 · 使用上次规则" : "等待连接坐功",
    detail: state.connection === "cache"
      ? `${reason}${state.active ? state.hostCount > 0 ? `暂时沿用 ${counts}；请检查主程序并重试同步。` : `暂时屏蔽 ${state.ruleCount} 个网址；请检查主程序并重试同步。` : "当前没有生效规则。"}`
      : `${reason}请启动本机的坐功主程序，然后重试。首次成功同步前无法屏蔽。`,
  };
}
