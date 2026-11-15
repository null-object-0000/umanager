const copyButton = document.querySelector('#copy-install');
copyButton?.addEventListener('click', async () => {
  const status = document.querySelector('#copy-status');
  try {
    await navigator.clipboard.writeText(document.querySelector('#install-command').textContent.trim());
    status.textContent = '命令已复制。请在安装包所在目录运行。';
    copyButton.textContent = '已复制 ✓';
  } catch { status.textContent = '未能访问剪贴板，请选中上方命令手动复制。'; }
});

document.querySelector('#reset-demo')?.addEventListener('click', () => {
  const frame = document.querySelector('#product-demo');
  if (frame instanceof HTMLIFrameElement) frame.src = '/demo/';
});
