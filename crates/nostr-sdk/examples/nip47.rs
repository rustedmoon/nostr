use std::str::FromStr;

use nostr_sdk::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let mut nwc_uri_string = String::new();
    let mut invoice = String::new();

    println!("Please enter a NWC string");
    std::io::stdin()
        .read_line(&mut nwc_uri_string)
        .expect("Failed to read line");

    println!("Please enter a BOLT 11 invoice");
    std::io::stdin()
        .read_line(&mut invoice)
        .expect("Failed to read line");

    invoice = String::from(invoice.trim());

    let nwc_uri =
        NostrWalletConnectURI::from_str(&nwc_uri_string).expect("Failed to parse NWC URI");

    let my_keys = Keys::new(nwc_uri.secret);

    let client = Client::new(&my_keys);
    client.add_relay(nwc_uri.relay_url.clone()).await?;

    client.connect().await;
    println!("Connected to relay {}", nwc_uri.relay_url);

    let req = nip47::Request {
        method: Method::PayInvoice,
        params: RequestParams::PayInvoice(PayInvoiceRequestParams { invoice }),
    };

    let encrypted = nip04::encrypt(&nwc_uri.secret, &nwc_uri.public_key, req.as_json()).unwrap();
    let p_tag = Tag::public_key(nwc_uri.public_key);

    let req_event = EventBuilder::new(Kind::WalletConnectRequest, encrypted, [p_tag])
        .to_event(&Keys::new(nwc_uri.secret))
        .unwrap();

    let subscription = Filter::new()
        .author(nwc_uri.public_key)
        .kind(Kind::WalletConnectResponse)
        .event(req_event.id)
        .since(Timestamp::now());

    client.subscribe(vec![subscription]).await;

    client.send_event(req_event).await.unwrap();

    client
        .handle_notifications(|notification| async {
            if let RelayPoolNotification::Event { event, .. } = notification {
                let decrypt_res =
                    nip04::decrypt(&nwc_uri.secret, &nwc_uri.public_key, &event.content).unwrap();
                println!("{:?}", decrypt_res);

                let nip47_res = nip47::Response::from_json(decrypt_res).unwrap();

                if let Some(ResponseResult::PayInvoice(pay_invoice_result)) = nip47_res.result {
                    println!("Payment sent. Preimage: {}", pay_invoice_result.preimage);
                } else {
                    println!("Unexpected result: {:?}", nip47_res.as_json());
                }
            }
            Ok(true)
        })
        .await?;

    Ok(())
}
