# The dark factory

Lights-out manufacturing is a real thing. A line runs with the lights off because nobody's standing on it. Material goes in one end, finished product comes out the other, and the machines handle the middle without a human watching each station. "Lights-out" is the industry term for an unstaffed automated line.

That's the model darkrun is built on. It's where the name comes from, and it's what `dark` mode is.

## A factory is an ordered line of stations

A factory floor runs as a sequence, not a pile of machines. Each station does one thing, hands the product to the next, and the order is the whole point. You don't paint before you weld.

darkrun's factory works the same way. An ordered line of stations, each retiring one class of risk, handing a locked artifact to the next.

:::callout
The order follows one rule: **cost of late discovery**. Cheapest mistakes to catch go first, because a mistake caught early costs almost nothing and the same mistake caught late costs a rewrite.
:::

## The line, in order

| Station | Kills | Caught late, it costs you |
|---|---|---|
| **Frame** | the wrong thing | a finished feature nobody wanted |
| **Specify** | ambiguity | a build that's technically correct and actually wrong |
| **Shape** | expensive structural reversal | ripping out an architecture under load-bearing code |
| **Build** | implementation defects | bugs shipped into review and beyond |
| **Prove** | escaped defects | a regression your tests should have caught |
| **Harden** | works-in-dev-dies-in-prod | the 2am page |

Read it top to bottom.

:::keypoints title="Why this order"
- Frame is first because building the wrong thing perfectly is the most expensive mistake there is, and it's free to catch — you just have to ask before you start.
- Specify kills the ambiguity that turns into "that's not what I meant" three stations later.
- Shape locks structure before there's code stacked on top of it, when reversing a decision is still cheap.
- By the time you're in Build, the expensive questions are answered and the agent can implement against a settled spec.
:::

Prove and Harden are the back half. Prove catches the defect that escaped Build. Harden catches the thing that works on your laptop and dies in production. Each one is a net under the last.

## Each station locks one artifact

The thing that makes the order hold is that stations don't reopen each other's work. Frame produces a frame, and once it's locked, Specify can't relitigate what we're building — it works inside that frame. Shape locks the structure; Build can't redesign it. Each station retires its risk, writes down the result, and seals it. Downstream consumes the artifact and can't reach back to change it.

That's what keeps a long run from thrashing. Without locked artifacts, every station is free to second-guess every earlier decision, and the run never converges. The lock is the commitment. It's what lets the line run forward instead of in circles.

```
Frame ─▶ Specify ─▶ Shape ─▶ Build ─▶ Prove ─▶ Harden
  │         │         │        │        │         │
 lock      lock      lock     lock     lock      lock
```

## Lights-out is the dark mode

A staffed line has a human at every station signing off before the product moves. That's `solo` and `team` mode — you or your team look at each boundary before the run crosses it.

`dark` mode is the lights-out line. You frame the work once, up front, in full. Then the run goes station to station with nobody standing on the floor. It only stops for the two things an unstaffed line genuinely can't handle on its own: a gate that's waiting on something external, and real ambiguity it can't resolve from the frame. Otherwise the lights stay off and the product moves.

That only works because the stations are real. A lights-out factory is only safe if each station actually retires its risk before passing the product on. The whole design — order by cost of late discovery, lock one artifact per station, never reopen — is what makes it safe to turn the lights off.

:::callout
Build the line so each station is honest. Then you can leave the floor.
:::
