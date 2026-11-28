# Snake: mechanics sheet

There is no original to measure: this is an original game built from the request.
Every value below is a design choice and is marked **estimated**; the request itself
is the source for the controls and features it names.

## Board
| Value | Number | Source |
|---|---|---|
| Grid size | 24 x 24 cells | estimated |
| Cell size | 24 px (board is 576 x 576 px) | estimated |
| Walls | solid border: hitting it ends the game | estimated |

## Snake
| Value | Number | Source |
|---|---|---|
| Starting length | 3 cells | estimated |
| Starting position | head at cell (8, 12), facing right | estimated |
| Growth per food | +1 cell | estimated |
| Self-collision | ends the game (moving into the cell the tail is leaving is allowed) | estimated |
| Reverse turn | ignored (can't turn straight back into yourself) | estimated |
| Turn buffer | up to 2 queued turns, so quick double-taps register | estimated |

## Speed
| Value | Number | Source |
|---|---|---|
| Starting step time | 140 ms per cell (about 7 cells/s) | estimated |
| Speed-up per food | step time -5 ms | request ("speed-up"), amount estimated |
| Fastest step time | 55 ms per cell (about 18 cells/s) | estimated |
| Level shown on screen | 1 + foods eaten / 5 | estimated |

## Scoring
| Value | Number | Source |
|---|---|---|
| Points per food | 10 | estimated |
| Best score | kept in the browser (localStorage) | estimated |
| Food placement | random empty cell | estimated |

## Controls (from the request)
| Action | Keys |
|---|---|
| Move | Arrow keys or W A S D |
| Start | any move key, Space or Enter (or click/tap the board) |
| Pause / resume | P or Esc |
| Restart after game over | Space, Enter or R (or click/tap), allowed 300 ms after dying (estimated) |

## Screens and on-screen information
- Top bar: score (left), level (centre), best (right).
- Start screen: title and "press an arrow key or WASD to start".
- Pause screen: "Paused".
- Game over: "Game over", final score, "new best" when beaten, how to restart.

## Check hooks (for Craft Anything)
The game is the single `index.html`. `main.py` is only a launcher: without options it opens the
page in the web browser; `--screenshot <png>` and `--self-test <json>` load the page in a hidden
(headless) Chromium-family browser with the hooks below and save the result.

- `index.html?screenshot` starts a game with the snake moving on its own for about 120 frames, then sets `window.__screenshotReady = true`.
- `index.html?self-test` drives the snake with scripted key presses and writes the result JSON
  into `window.__selfTestResult` and into the `#self-test-result` element.
