# The methodology

darkrun is built on one bet: the cost of a mistake grows the later you find it.
A wrong assumption in Frame is cheap. The same wrong assumption discovered in
Harden is a rewrite. So the line is ordered to surface risk early and pay for
discovery while it is still cheap.

## Cost of late discovery

Every station is placed by how expensive its class of mistake becomes downstream.
Frame eliminates "we built the wrong thing." Specify eliminates "we disagreed
about done." Shape eliminates "the design can't hold." Build, Prove, and Harden
turn a sound design into shipped, evidenced, signed work. You move forward only
when a station's risk is retired.

## The manager

The **manager** is the loop that runs the line. It advances each station through
its phases, dispatches workers, collects reviewer findings, and stops at the
checkpoints. It is deterministic about sequence and honest about state: a station
cannot lock its artifact until its audit passes — and the audit folds in the
quality checks, so passing means the reviewers signed off *and* the tests, types,
and lints are green.

## Humans at the gates

The point of the phase machine is to let a human spend attention where it counts
— at the checkpoints — instead of babysitting every keystroke. Auto checkpoints
keep low-risk stations moving. Ask, External, and Await checkpoints pull you in
exactly when judgment, sign-off, or a long task demands it.
