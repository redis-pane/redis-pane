# Terminal sizes

`redis-pane` fits the terminal it's in, down to 80 columns by 24 rows, and works below that with
less on screen.

## Width

| Width | Layout |
| --- | --- |
| 120 columns and up | Two panes. The key list shows `KEY`, `TYPE`, `SIZE` and `TTL`. |
| 90 to 119 | Two panes. The key list drops `SIZE`. |
| 70 to 89 | Two panes, tight. The title bar shortens the target from the left, and never the Environment or the Source. |
| Under 70 | One pane at a time. |

`TTL` is the last column to go, because it's the one you want when the screen is small and the
situation is urgent.

**Under 70 columns**, opening a key (`→`) replaces the key list with the value, under a
breadcrumb like `Esc back · key-name`. `Esc` goes back. Focus is the same idea at every width:
it decides which pane the pane-specific keys act on.

## Height

Under 24 rows, the hint bar folds into the status bar.

## Moving the divider

`⌃←` and `⌃→` narrow and widen the key list. Your adjustment is remembered for the session and
saved in [session restore](../browsing/session-restore.md). If you resize the terminal past a
breakpoint, the columns rearrange as above and your adjustment stays.

## The server views

The Dashboard has its own grid: three tiles across at 120 columns and up, two at 80 to 119, one
below that. Monitor, Pub/Sub and Slowlog each drop their least important columns first as the
terminal narrows.

## The mouse

The mouse is never required. Where it works, a click focuses a pane, the wheel scrolls the pane
under it, and dragging the divider resizes. All of it is on the keyboard too.
