## Usage
Send `@roles` to get a help message.

Sending a message that starts with `@roles` and contains one or more `{ROLE:id_or_name}`
will start a reaction message setup.
`@roles You can put text anywhere {ROLE:Blue} in the message.`

## Docker Setup
```fish
mkdir data
sudo docker run --user 1000:1000 -e "BOT_TOKEN=" -v ./data:/data bobbobs/roles-bot
```
The `data` folder will be used to store the `roles.sqlite` file for autorole settings.
