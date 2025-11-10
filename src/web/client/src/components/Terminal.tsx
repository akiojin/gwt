/**
 * Terminal Component
 *
 * xterm.jsを使用したブラウザ端末エミュレータ。
 * WebSocket経由でPTYプロセスと通信します。
 */

import React, { useEffect, useRef } from "react";
import { Terminal as XTerm } from "xterm";
import { FitAddon } from "xterm-addon-fit";
import { PTYWebSocket } from "../lib/websocket";
import "xterm/css/xterm.css";

export interface TerminalProps {
  sessionId: string;
  onExit?: (code: number) => void;
  onError?: (message: string) => void;
}

export function Terminal({ sessionId, onExit, onError }: TerminalProps) {
  const terminalRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const wsRef = useRef<PTYWebSocket | null>(null);

  useEffect(() => {
    if (!terminalRef.current) {
      return;
    }

    // xterm.jsのインスタンスを作成
    const xterm = new XTerm({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: '"Cascadia Code", "SF Mono", Monaco, Consolas, monospace',
      theme: {
        background: "#1e1e1e",
        foreground: "#d4d4d4",
        cursor: "#aeafad",
        black: "#000000",
        red: "#cd3131",
        green: "#0dbc79",
        yellow: "#e5e510",
        blue: "#2472c8",
        magenta: "#bc3fbc",
        cyan: "#11a8cd",
        white: "#e5e5e5",
        brightBlack: "#666666",
        brightRed: "#f14c4c",
        brightGreen: "#23d18b",
        brightYellow: "#f5f543",
        brightBlue: "#3b8eea",
        brightMagenta: "#d670d6",
        brightCyan: "#29b8db",
        brightWhite: "#ffffff",
      },
    });

    // FitAddonを追加
    const fitAddon = new FitAddon();
    xterm.loadAddon(fitAddon);

    // ターミナルをDOMにマウント
    xterm.open(terminalRef.current);
    fitAddon.fit();

    xtermRef.current = xterm;
    fitAddonRef.current = fitAddon;

    // WebSocket接続を確立
    const ws = new PTYWebSocket(sessionId, {
      onOutput: (data) => {
        xterm.write(data);
      },
      onExit: (code) => {
        xterm.write(`\r\n\r\n[Process exited with code ${code}]\r\n`);
        onExit?.(code);
      },
      onError: (message) => {
        xterm.write(`\r\n\r\n[Error: ${message}]\r\n`);
        onError?.(message);
      },
      onOpen: () => {
        xterm.write("Connected to session...\r\n");
      },
      onClose: () => {
        xterm.write("\r\n[Connection closed]\r\n");
      },
    });

    ws.connect();
    wsRef.current = ws;

    // ユーザー入力をWebSocketに送信
    xterm.onData((data) => {
      ws.sendInput(data);
    });

    // ウィンドウリサイズ時にターミナルサイズを調整
    const handleResize = () => {
      fitAddon.fit();
      if (ws.isConnected()) {
        ws.sendResize(xterm.cols, xterm.rows);
      }
    };

    window.addEventListener("resize", handleResize);

    // クリーンアップ
    return () => {
      window.removeEventListener("resize", handleResize);
      ws.disconnect();
      xterm.dispose();
    };
  }, [sessionId, onExit, onError]);

  return (
    <div
      ref={terminalRef}
      style={{
        width: "100%",
        height: "100%",
        padding: "8px",
        backgroundColor: "#1e1e1e",
      }}
    />
  );
}
