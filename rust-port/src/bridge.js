/*
 * 前后端桥接层（注入到页面上下文，早于页面脚本执行）。
 *
 * 作用：把 Rust 端暴露的 RPC 方法挂到 window.go.main.App 上，
 * 与 Wails 生成的绑定保持完全相同的调用约定：
 *
 *     const list = await window.go.main.App.GetCredentials(-1, "");
 *
 * 调用失败时 reject 一个 Error，`String(err)` 直接就是错误文案（与 Wails 行为一致）。
 */
(function () {
    'use strict';

    if (window.__vaultBridgeInstalled) {
        return;
    }
    window.__vaultBridgeInstalled = true;

    var seq = 0;
    var pending = Object.create(null);

    function BridgeError(message) {
        this.name = 'BridgeError';
        this.message = String(message);
    }
    BridgeError.prototype = Object.create(Error.prototype);
    BridgeError.prototype.constructor = BridgeError;
    BridgeError.prototype.toString = function () {
        return this.message;
    };

    function invoke(method, args) {
        return new Promise(function (resolve, reject) {
            var id = ++seq;
            pending[id] = { resolve: resolve, reject: reject };
            try {
                window.ipc.postMessage(JSON.stringify({ id: id, method: method, args: args }));
            } catch (e) {
                delete pending[id];
                reject(new BridgeError('与后端通信失败: ' + e));
            }
        });
    }

    /** Rust 侧回填结果：ok=true 时 payload 为返回值，否则为错误文案 */
    window.__vaultBridgeSettle = function (id, ok, payload) {
        var entry = pending[id];
        if (!entry) {
            return;
        }
        delete pending[id];
        if (ok) {
            entry.resolve(payload);
        } else {
            entry.reject(new BridgeError(payload));
        }
    };

    var METHODS = /*__METHODS__*/ [];

    var app = {};
    METHODS.forEach(function (name) {
        app[name] = function () {
            return invoke(name, Array.prototype.slice.call(arguments));
        };
    });

    window.go = window.go || {};
    window.go.main = window.go.main || {};
    window.go.main.App = app;

    // 便捷别名，便于在 WebView 控制台里手工调试
    window.__vaultCall = invoke;
})();
