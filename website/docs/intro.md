---
slug: /
sidebar_position: 1
title: Introduction
---

# Introduction

An STM32F4 microcontroller emulator that runs **real Cortex-M4F firmware**
in the browser or Node.js. No SDL, no native code, no hardware required.

## What is it?

- **Rust Thumb-2 CPU core** (`stm32-periph-wasm/src/cpu/`) — Cortex-M4F
  interpreter with exact exception entry/return, compiled to WASM
- **Rust peripheral model** — GPIO, USART, SPI, I2C, DMA, ETH, TIM, CAN,
  ADC, DAC, USB, LTDC, DMA2D and more, same WASM module
- **JavaScript drivers** — Node CLI, browser console with live UART/GPIO/
  packet viewer, WebSocket bridge for headless runs

The same firmware binaries that run here also run on real F4 hardware.

## Try it

Open the [live console](https://danish9661.github.io/stm32F4-emulator/console/console.html) — pick a firmware preset
(`?fw=blinky`), a board (`?board=stm32f401`), or upload your own
`.bin` / `.hex` / `.elf`.

## Boards

Five chips on one shared core: STM32F401, F411, F407, F407VE/ZE, F429.
See [Boards & chips](boards.md).
