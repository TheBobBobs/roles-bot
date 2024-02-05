pub const HELP_MESSAGE: &str = 
"Bot needs `AssignRoles` and `React` permissions!
The bot can only assign roles lower than it's highest role.
If a user is ranked above the bot it cannot give them roles.

Edit role colours:
<@01G9XW2NR0QBH5SD3RMDX7VWDB> colour

Create a reaction message:
<@01G9XW2NR0QBH5SD3RMDX7VWDB> `{ROLE:Rust}` the bot will replace this in the next step.
You can put roles anywhere `{ROLE:Python}` in the message


You can then react to the message with the emojis you want.
When you're done react with the checkmark.

The above example would look like this when done:

:01GAR9TW0FGMH680JM2C0P0Y02:[](ROLE_ID) __Rust__ the bot will replace this in the next step.
You can put roles anywhere :01GAR81WB2HZQ4DZTQ0MWCFHJC:[](ROLE_ID) __Python__ in the message [](https://autumn.revolt.chat/attachments/7Qvk6O4PmvPwLblX_cx_PVrCTeaXNEL1BtkvVg1UOQ/roles.mp4)";

pub const HELP_COLOUR_MESSAGE: &str =
"Set or clear A roles color.
Usage
<@01G9XW2NR0QBH5SD3RMDX7VWDB> color `ROLE NAME or ID` `COLOR`

Color can be by name(`red`) or hex(`#C10417`)
Use 2 or more colors for gradients

Custom colors can also be used
`linear-gradient(30deg, purple, orange)`";