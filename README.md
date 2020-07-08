# wealthsimple-prometheus

> *Please note*: Do _not_ run this facing the internet, unless you're fine with
> the whole world seeing how much money you've got in the bank.

This is a pretty simple service thing that polls [Wealthsimple] for account data
and makes it available for [Prometheus] to pull it in.

[Wealthsimple]: https://wealthsimple.com
[Prometheus]: https://prometheus.io

Currently, there are four [gauges]:

- `wealthsimple_deposited`: The total amount of money that has been deposited
- `wealthsimple_withdrawn`: The total amount of money that has been withdrawn
- `wealthsimple_net_liquidation`: The value were all assets to be liquidated
- `wealthsimple_gross_position`: The sum of all positions

Each of these gauges is tagged with the following:

- `account_id`: The account identifier within Wealthsimple
- `account_type`: The type of account
- `account_name`: The nickname of the account (this one seems to not work)

[gauges]: https://prometheus.io/docs/concepts/metric_types/#gauge

This is only scratching the surface, though. Take a look at the [api docs] to
see some of the other cool stuff that maybe works.

[dev docs]: https://developers.wealthsimple.com/

## Usage

To use this, all you need to do is clone the repo and run `cargo run` inside it.
You could use a release build, but I don't think there's much concern about
performance here... (it spends most of the time sleeping).

When you run it, it'll prompt you for your credentials. These aren't stored
anywhere (which means you'll have to enter it every time you restart it), and
the password prompt is an actual password prompt (the text you type is not
displayed). If you have 2FA set up (which you probably should), you'll be
prompted for a one-time password as well. You shouldn't have to enter the a 2FA
code again for the life of the program (unless it runs more than 30 days), since
it passes the same `remember=true` as the actual Wealthsimple browser client.

Once it's running, you can point Prometheus at it. Currently it always just runs
on port 8080.

This is how I've got it configured:

```yaml
scrape_configs:
  - job_name: 'wealthsimple'
    static_configs:
    - targets: ['localhost:8080']
```

## How it works

There are two threads: One to poll Wealthsimple forever, and one to serve
requests to the `/metrics` resource forever.

I've avoided `unwrap()`-ing anything so that this will be reliable, but there
are plenty of places where errors are propagated, which will instantly
terminate the program. We'll consider improving this a TODO.

In the polling loop, the following happens:

1. A request is made to `https://api.production.wealthsimple.com/v1/accounts` to
   get a list of the user's accounts.
1. If the request came back as a 401, the auth token probably expired, so get a
   new one and try again.
1. Some choice information is shoved into the gauges we mentioned earlier.
1. Sleep for 5 minutes.

The server loop is even simpler:

1. Get the next request.
1. Return a 404 status if it's not for `/metrics`.
1. Encode the gauge data into Prometheus-speech.
1. Send it back.

## Why?

I'm not a fan of how Wealthsimple graphs your accounts by default. I don't
really care that the value of my account has increased due to my deposits; it
just makes it more difficult to read the changes in returns.

Keep in mind I'm no [financier], I mostly just like pretty graphs. If you're
anything like me, this is the query you're looking for (probably; I'm a data
noob):

```promql
(((wealthsimple_net_liquidation - wealthsimple_withdrawn) / (wealthsimple_deposited - wealthsimple_withdrawn)) - 1) * 100
```

[financier]: https://en.wikipedia.org/wiki/Financier_(cake)
