
> **Run** `darkrun-sim` · **Phase** `noop`


# Hold — nothing to dispatch

Mid-wave noop. Outstanding unit passes are still in flight — wait, then retick.

Do **not** invent work to fill the gap. Let the in-flight work finish, then call `darkrun_tick` again for the next real action.