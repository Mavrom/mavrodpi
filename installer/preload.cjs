"use strict";

const { contextBridge, ipcRenderer } = require("electron");

const api = Object.freeze({
  getInfo: () => ipcRenderer.invoke("installer:get-info"),
  install: (launchAfterInstall) =>
    ipcRenderer.invoke("installer:install", {
      launchAfterInstall: launchAfterInstall === true,
    }),
  launch: () => ipcRenderer.invoke("installer:launch"),
  close: () => ipcRenderer.invoke("window:close"),
  onStatus: (callback) => {
    if (typeof callback !== "function") {
      return () => {};
    }

    const listener = (_event, status) => callback(status);
    ipcRenderer.on("installer:status", listener);
    return () => ipcRenderer.removeListener("installer:status", listener);
  },
});

contextBridge.exposeInMainWorld("mavroInstaller", api);
