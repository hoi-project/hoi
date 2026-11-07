// The client's own notices: what every connection shows, said before the first one
// is made, and who made this software. The words are data here so the screens, the setup app
// and the tests read the same text. Names and contact come from the build (HOI_PUBLISHER,
// HOI_CONTACT), never from the source: public copy carries no addresses.
import { NETWORK_NOTICE_VERSION, PRIVACY_NOTICE_DATE, PRIVACY_NOTICE_VERSION } from './version.mjs';

export const APP = 'hoi';
/** Who publishes this build: a name or a project, from HOI_PUBLISHER at build time. */
export const PUBLISHER: string = (typeof __HOI_PUBLISHER__ === 'string' && __HOI_PUBLISHER__.trim()) || 'the hoi project';
/** How to reach them; empty means "through the source repository". */
export const CONTACT: string = (typeof __HOI_CONTACT__ === 'string' && __HOI_CONTACT__.trim()) || '';
export const CONTACT_LINE = CONTACT || 'through the source repository (Source, in the footer)';

export interface LegalSection {
  title: string;
  text: string;
  bullets?: string[];
  after?: string;
}

/** Shown before the first connection, and again when its version changes. */
export const NETWORK_SAFETY = {
  version: NETWORK_NOTICE_VERSION,
  title: 'Network & Safety',
  sections: [
    {
      title: 'You are joining a peer-to-peer network.',
      text: `Your device will connect directly to other participants and to nodes run by independent operators. This is how ${APP} works, and it cannot be disabled.`,
    },
    {
      title: 'Your connections do not stay with one node.',
      text: 'The node or invite you start with is only your way in. Joining a community, opening an invite or a link, reading, posting, replying and syncing can connect your device to further nodes and participants anywhere on the internet, picked by the network rather than by you, and which ones changes over time. Public nodes also tell your device about other public nodes, and it may connect to those on its own.',
    },
    {
      title: 'Your posts travel in pieces.',
      text: 'In most communities what you post is sealed on your device, cut into pieces and sent through public nodes to many independent nodes that keep one piece each; members rebuild it on their own devices. The nodes that keep the pieces cannot read them and do not see your address. The public node your device sends through does see your address and that you sent something, though not what. Open communities stay public: anyone who joins can read what you post there.',
    },
    {
      title: 'Your IP address will be visible.',
      text: 'Anyone you connect with can see your public IP address. An IP address can reveal your approximate location and, together with other information, your identity.',
    },
    {
      title: 'Not everyone may act in good faith.',
      text: 'Most participants and operators use the network as intended. However, anyone can run a node or join a community, and a malicious actor could log IP addresses, analyse connection patterns or attempt attacks on your connection. Neither we nor the node operator can fully prevent this.',
    },
    {
      title: 'Use a VPN at all times.',
      text: `We strongly recommend running a trusted VPN whenever ${APP} is open. Please note:`,
      bullets: [
        'Start the VPN before opening the client, not afterwards.',
        'Make sure your VPN also routes WebRTC traffic. Some browsers can leak your real IP address through WebRTC even with a VPN active. Test this with a WebRTC leak test.',
        'A VPN hides your IP address from peers, but your VPN provider can see your traffic. Choose one you trust.',
        'A VPN does not protect what you publish yourself. Do not share personal information you would not want strangers to have.',
      ],
    },
  ] satisfies LegalSection[],
  checks: {
    ip: 'I understand that my IP address and connection data will be shared with other participants and nodes.',
    vpn: 'I understand that I should use a VPN to protect myself.',
  },
  cta: 'Continue',
};

/** The client's privacy notice: who made it, what the network shares, what stays on the device. */
export const PRIVACY_NOTICE = {
  version: PRIVACY_NOTICE_VERSION,
  date: PRIVACY_NOTICE_DATE,
  title: `Privacy Notice – ${APP} client`,
  sections: [
    {
      title: 'Who we are.',
      text: `${APP} is open-source software developed by ${PUBLISHER}. Contact: ${CONTACT_LINE}. We provide the software only. We do not operate the network, we do not run the nodes you join, and we do not receive your messages or connection data.`,
    },
    {
      title: 'How the network works.',
      text: `${APP} is a peer-to-peer network based on WebRTC. When you join a community, your device connects directly to public nodes and to other participants. There is no central server operated by us.`,
    },
    {
      title: 'Where your device connects.',
      text: 'The first node or invite you use is only your way in; it does not limit where your device goes afterwards. Once you are in, your device connects to whichever nodes and participants carry the communities you take part in:',
      bullets: [
        'joining a community, opening an invite or a node link, reading, posting, replying, voting and syncing can each connect it to further nodes and devices anywhere on the internet;',
        'public nodes tell your device about other public nodes, and it may connect to those on its own;',
        'when you found a community or make an invite for it, your device asks public nodes to carry it, and they pass it on to further public nodes.',
      ],
      after:
        "Which nodes and devices these are changes over time, without you choosing each one. A public node that carries a join notice shows it to you before your device tells it anything about you. Participants' devices, and nodes without a notice, show none, and like every node they see your IP address as soon as the connection is made.",
    },
    {
      title: 'What is always shared.',
      text: 'By joining any community, you will exchange data with other participants and nodes. This is technically required and cannot be turned off. At a minimum this includes:',
      bullets: [
        'your public IP address and network information needed to establish connections;',
        'the time and duration of your connections;',
        'technical data such as client version and supported protocol version;',
        'everything you choose to publish or send in a community.',
      ],
      after:
        'What you publish in an open community is not encrypted against its members: anyone who joins can read it, and members’ devices keep the copies they read. In communities that keep posts in pieces (every new one, unless its owners chose otherwise), nodes that carry the community receive a small pointer signed by you, and the post itself is kept as sealed pieces on many independent nodes, each holding one and able to read none; where too few such nodes are around, or in older communities, every node and member that carries the community receives a whole copy, including nodes your device never connected to. Private communities are encrypted for their members only. Other participants and node operators may see, log or store what reaches them. They may be located anywhere in the world, under any law, including places with little or no data protection.',
    },
    {
      title: 'What the nodes in between see.',
      text: 'Pieces and private messages reach the nodes that keep them through public nodes that pass them on. The node your device connects through sees your IP address, when you send or fetch something and how much, but not what: everything it passes is sealed to the node that keeps it. The nodes that keep pieces see the node that passed them on, not you. A public node may itself keep pieces for others, and its join notice says so.',
    },
    {
      title: 'Who is responsible.',
      text: 'Each public node is run by an independent operator. That operator is responsible for the data processed on their node and is asked to tell you about it in a join notice. You see that notice when you open their node link, and the first time your device meets their node, before anything of yours is sent to it. Operators are asked, not forced, to say who they are, how to reach them and where their server is: a notice that leaves any of this out says so, and you can decline it. We have no control over nodes, operators or other participants.',
    },
    {
      title: 'Data stored on your device.',
      text: `The client stores your identity keys, your communities with their posts and media (including those it rebuilt from pieces, with what it needs to find and repair them), your settings, the pairing credential for your own home node, the public nodes it met with their operators' notices and your answers to them, and the community listings it heard of, in an encrypted database in this browser's storage (IndexedDB), under the passphrase you choose. The theme, the motion setting and your acknowledgement of the Network & Safety notice are kept beside it, unencrypted. This data does not leave your device unless it is needed for the network to function. You can delete it at any time under You → On this ${'{device}'} → Wipe, or with “Clear this device's local data” on the unlock screen.`,
    },
    {
      title: 'No tracking.',
      text: 'The client contains no analytics, no advertising and no third-party services. The release notice at the top of the app comes from the site that served it. Where whoever hosts a build configured a public registry (for community names) or a relay for private communities, the client contacts them only after you accepted the Network & Safety notice; that host names them in its own privacy policy.',
    },
    {
      title: 'Your protection.',
      text: `We strongly recommend using a trusted VPN at all times while using ${APP}. See Network & Safety for details.`,
    },
    {
      title: 'Your rights.',
      text: `For data processed on a node, contact its operator through the contact in its join notice. A node whose operator gave no contact offers no way to ask; you can decline it and use another. For questions about this software, contact ${CONTACT_LINE}.`,
    },
    {
      title: 'Changes.',
      text: `If this notice changes, the client shows the new version before you continue. Version ${PRIVACY_NOTICE_VERSION}, ${PRIVACY_NOTICE_DATE}.`,
    },
  ] satisfies LegalSection[],
};

/** Shown wherever someone becomes an operator: the kit page, the setup app, the download page.
 *  No country and no particular law: which rules apply depends on where the operator, the server
 *  and the people who connect are, and the words say so. */
export const OPERATOR_RESPONSIBILITY = {
  title: 'Before you run a public node',
  sections: [
    {
      title: 'You become responsible.',
      text: 'When you run a public node and invite others, you process their personal data, at least their IP addresses and connection metadata. Under the data protection rules that apply where you, your server and those people are, whoever runs such a node is usually the one responsible for that data (often called the data controller). That is you. We, the developers, are not.',
    },
    {
      title: 'Private use has narrow limits.',
      text: 'Many data protection rules exempt purely personal or household use, such as a node only your family and close friends use. A community open to people you do not know personally, even without any commercial purpose, generally does not count as personal use.',
    },
    {
      title: 'What you must do as an operator:',
      text: '',
      bullets: [
        'Write a join notice. The installer lets you define it, and every user must accept it before your node serves them.',
        'Explain in it what your node stores and for how long, including what your server or your hosting provider logs.',
        'Keep your server secure and up to date.',
        'Respond to requests from users about their data.',
      ],
    },
    {
      title: 'Who you are: optional, but you should say.',
      text: 'The join notice asks for your name or a pseudonym, a way to reach you, the country your server is in and who hosts it. You can leave any of them empty; the notice then tells people plainly that you did, and that they cannot reach you about their data through it. You should give them: they tell people who answers for their data and how to ask, and many data protection rules expect an operator to be identifiable and reachable. Whatever you fill in is shown to everyone who connects to your node, and travels in its node link, so anyone who has the link can read it.',
    },
    {
      title: 'What the software does for you.',
      text: 'The client always shows the standard network and VPN warning. Your join notice is added on top and cannot be skipped by users. A public node keeps encrypted pieces and private communities’ mailboxes for other people’s devices, up to 2 GB unless you choose another amount or none; it can read none of them, and the software adds a fixed sentence saying so to your notice.',
    },
    {
      title: 'This is not legal advice.',
      text: 'Which rules apply depends on where you, your server and the people who connect are, and more than one set may apply at once. If you run a large or organised community, check your obligations with a qualified professional.',
    },
  ] satisfies LegalSection[],
  check: 'I understand that I am responsible for the data processed on my node.',
};
