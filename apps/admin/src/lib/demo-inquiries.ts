import type {
  Inquiry,
  InquiryFile,
  InquiryForm,
  InquiryFormRevision,
  NotificationChannel,
  NotificationChannelDelivery,
  NotificationChannelEvent,
} from "@fastcart/shared-types";

const NOW = Math.floor(new Date("2026-11-04T12:00:00Z").getTime() / 1000);

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

const FORM_ID = "iform_demo_contact";
const REVISION_V1 = "ifrev_demo_contact_1";
const REVISION_V2 = "ifrev_demo_contact_2";

const form: InquiryForm = {
  id: FORM_ID,
  name: "Contact us",
  handle: "contact",
  description: "Questions about an order, a product or a wholesale enquiry.",
  status: "published",
  current_version: 2,
  published_version: 1,
  created_at: NOW - 60 * 86_400,
  updated_at: NOW - 3 * 86_400,
};

const revisionV1: InquiryFormRevision = {
  id: REVISION_V1,
  form_id: FORM_ID,
  version: 1,
  fields: [
    {
      key: "name",
      kind: "text",
      label: "Name",
      required: true,
      placeholder: "How should we address you?",
      help: null,
      default: null,
      options: [],
      validation: { min_length: 1, max_length: 100 },
      file: null,
    },
    {
      key: "email",
      kind: "email",
      label: "Email",
      required: true,
      placeholder: "you@example.com",
      help: null,
      default: null,
      options: [],
      validation: {},
      file: null,
    },
    {
      key: "subject",
      kind: "text",
      label: "Subject",
      required: false,
      placeholder: "What can we help with?",
      help: null,
      default: null,
      options: [],
      validation: { max_length: 160 },
      file: null,
    },
    {
      key: "message",
      kind: "textarea",
      label: "Message",
      required: true,
      placeholder: "Tell us a little more",
      help: null,
      default: null,
      options: [],
      validation: { max_length: 2000 },
      file: null,
    },
    {
      key: "attachments",
      kind: "file",
      label: "Attachment",
      required: false,
      placeholder: null,
      help: "JPG, PNG or PDF up to 10 MB.",
      default: null,
      options: [],
      validation: {},
      file: {
        max_files: 5,
        max_bytes: 10 * 1024 * 1024,
        accept: ["image/jpeg", "image/png", "application/pdf"],
      },
    },
  ],
  settings: {
    submit_label: "Send message",
    success_message: "Thanks — we have received your message and will be in touch shortly.",
    redirect_url: null,
    consent_text: "I agree to be contacted about this enquiry.",
    consent_required: false,
    allow_attachments: true,
    notification_fields: ["name", "email", "subject"],
  },
  created_by: null,
  created_at: NOW - 60 * 86_400,
  published_at: NOW - 60 * 86_400,
};

const revisionV2: InquiryFormRevision = {
  ...clone(revisionV1),
  id: REVISION_V2,
  version: 2,
  fields: [
    ...clone(revisionV1.fields),
    {
      key: "company",
      kind: "text",
      label: "Company",
      required: false,
      placeholder: "Company name",
      help: null,
      default: null,
      options: [],
      validation: { max_length: 120 },
      file: null,
    },
    {
      key: "budget",
      kind: "select",
      label: "Budget",
      required: false,
      placeholder: null,
      help: null,
      default: null,
      options: [
        { value: "under_500", label: "Under $500" },
        { value: "500_2000", label: "$500 – $2,000" },
        { value: "above_2000", label: "Above $2,000" },
      ],
      validation: {},
      file: null,
    },
  ],
  settings: {
    ...revisionV1.settings,
    notification_fields: ["name", "email", "subject", "company"],
  },
  published_at: null,
  created_at: NOW - 3 * 86_400,
};

function inquiry(
  id: string,
  status: Inquiry["status"],
  name: string,
  email: string,
  subject: string,
  message: string,
  data: Record<string, unknown>,
  createdAt: number,
): Inquiry {
  return {
    id,
    event_id: `evt_${id}`,
    form_id: FORM_ID,
    form_revision_id: REVISION_V1,
    form_version: 1,
    status,
    name,
    email,
    phone: null,
    subject,
    message,
    data,
    source: "contact_form",
    page_url: "https://halcyonsupply.com/pages/contact",
    locale: "en-US",
    idempotency_key: null,
    metadata: {},
    created_at: createdAt,
    updated_at: createdAt,
  };
}

const inquiries: Inquiry[] = [
  inquiry(
    "inq_demo_1",
    "new",
    "Mia Chen",
    "mia@studio-north.co",
    "Wholesale enquiry",
    "We are opening a small studio in Brooklyn and would love to stock a few pieces.",
    { company: "Studio North", budget: "500_2000" },
    NOW - 2 * 3_600,
  ),
  inquiry(
    "inq_demo_2",
    "read",
    "Theo Alvarez",
    "theo@example.com",
    "Sizing on the Ridge carafe",
    "Is the Ridge carafe dishwasher safe?",
    { company: "" },
    NOW - 27 * 3_600,
  ),
  inquiry(
    "inq_demo_3",
    "replied",
    "Amara Okafor",
    "amara@example.com",
    "Order #1042 delivery",
    "Could the delivery be left with the concierge?",
    { company: "Okafor Design" },
    NOW - 3 * 86_400,
  ),
  inquiry(
    "inq_demo_4",
    "closed",
    "Jonas Meyer",
    "jonas@example.com",
    "Press request",
    "Looking for high-resolution images of the Arc lamp.",
    { company: "Meyer Press" },
    NOW - 12 * 86_400,
  ),
];

const files: InquiryFile[] = [
  {
    id: "ifile_demo_1",
    inquiry_id: "inq_demo_1",
    field_key: "attachments",
    original_name: "studio-floorplan.pdf",
    content_type: "application/pdf",
    size_bytes: 284_112,
    checksum_sha256: "demo-checksum",
    status: "claimed",
    created_at: NOW - 2 * 3_600,
  },
];

const channel: NotificationChannel = {
  id: "nch_demo_feishu",
  kind: "feishu",
  name: "Store operations",
  events: ["order.placed", "inquiry.created"],
  is_active: true,
  config: { card_template: "compact" },
  endpoint_url_masked: "https://open.feishu.cn/open-apis/bot/v2/hook/••••3f2a",
  signing_secret_configured: true,
  last_error: null,
  last_success_at: NOW - 4 * 3_600,
  created_at: NOW - 40 * 86_400,
  updated_at: NOW - 4 * 86_400,
};

const deliveries: NotificationChannelDelivery[] = [
  {
    id: "ndel_demo_1",
    channel_id: channel.id,
    event_id: "evt_inq_demo_1",
    event_type: "inquiry.created",
    status: "sent",
    job_id: "job_demo_1",
    attempts: 1,
    response_status: 200,
    response_body: null,
    last_error: null,
    created_at: NOW - 2 * 3_600,
    updated_at: NOW - 2 * 3_600 + 4,
    sent_at: NOW - 2 * 3_600 + 4,
  },
  {
    id: "ndel_demo_2",
    channel_id: channel.id,
    event_id: "evt_order_demo_1",
    event_type: "order.placed",
    status: "failed",
    job_id: "job_demo_2",
    attempts: 3,
    response_status: 500,
    response_body: null,
    last_error: "Feishu responded with 500",
    created_at: NOW - 26 * 3_600,
    updated_at: NOW - 25 * 3_600,
    sent_at: null,
  },
];

const events: NotificationChannelEvent[] = [
  {
    type: "order.placed",
    label: "New order",
    group: "Orders",
    description: "Sent when an order is created.",
  },
  {
    type: "inquiry.created",
    label: "New inquiry",
    group: "Inquiries",
    description: "Sent when a storefront form is submitted.",
  },
];

export function demoInquiryForms(): InquiryForm[] {
  return [clone(form)];
}

export function demoInquiryForm(id: string): InquiryForm | null {
  return id === form.id ? clone(form) : null;
}

export function demoInquiryFormRevisions(formId: string): InquiryFormRevision[] {
  if (formId !== FORM_ID) return [];
  return [clone(revisionV2), clone(revisionV1)];
}

export function demoInquiries(): Inquiry[] {
  return clone(inquiries);
}

export function demoInquiryDetail(id: string): {
  inquiry: Inquiry;
  revision: InquiryFormRevision;
  files: InquiryFile[];
} | null {
  const found = inquiries.find((entry) => entry.id === id);
  if (!found) return null;
  return {
    inquiry: clone(found),
    revision: clone(revisionV1),
    files: clone(files.filter((file) => file.inquiry_id === id)),
  };
}

export function demoUnreadInquiries(): number {
  return inquiries.filter((entry) => entry.status === "new").length;
}

export function demoNotificationChannels(): NotificationChannel[] {
  return [clone(channel)];
}

export function demoNotificationEvents(): NotificationChannelEvent[] {
  return clone(events);
}

export function demoNotificationDeliveries(channelId: string): NotificationChannelDelivery[] {
  return channelId === channel.id ? clone(deliveries) : [];
}
